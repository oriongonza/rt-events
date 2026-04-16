# Algebraic Structure of rt-events

## Definitions

Let **E** be the set of all event types (`T: 'static`).

Let **S**(e) be the set of subscribers for event type e ∈ **E**.

Each subscriber sᵢ ∈ **S**(e) is a function sᵢ: &e → (). That is, an endomorphism on the unit type through the side-effect of observing e.

## Dispatch as Application

Emission of event e with subscribers {s₁, s₂, ..., sₙ} is:

    dispatch(e) = s₁(&e) ∘ s₂(&e) ∘ ... ∘ sₙ(&e)

where ∘ is sequential composition of side effects.

## Commutativity

**Claim.** If subscribers are independent (no shared mutable state between them), then for all sᵢ, sⱼ ∈ **S**(e):

    sᵢ(&e) ∘ sⱼ(&e) = sⱼ(&e) ∘ sᵢ(&e)

**Proof.** Independence means sᵢ and sⱼ mutate disjoint state. Disjoint mutations commute. ∎

**Consequence.** Under independence, **S**(e) forms a commutative monoid under composition with identity element id: &e → () (the no-op subscriber).

## What This Means

1. **Dispatch order is not a semantic property.** The Vec order of subscribers is therefore an implementation detail. `swap_remove` on unsubscribe is correct by commutativity — reordering cannot change observable behavior for independent subscribers.

2. **Parallel dispatch is sound.** Commutativity is the formal precondition for parallelizing dispatch without synchronization. Rust's ownership model enforces the independence condition at compile time: `&E` is immutable, and each callback's captured mutable state is disjoint (enforced by the borrow checker).

3. **Dispatch order is an optimization parameter.** Because order doesn't affect correctness, the runtime is free to reorder subscribers for cache locality, execution time, or any other performance criterion. This degree of freedom does not exist in ordered event systems.

## The Bus

The bus itself is a dependent product:

    EventBus = Πₑ∈E Vec<Subscriber(e)>

indexed by `TypeId`. Emission is pointwise application at a single index. The Vec<Vec<Subscriber>> layout is the flattened representation of this product.

## Delivery Guarantees

For an in-process, single-threaded bus:

- **At-most-once:** guaranteed. A subscriber is called at most once per emission.
- **At-least-once:** guaranteed (assuming no panic). Every registered subscriber is called.
- **Exactly-once:** guaranteed. Composition of the above.
- **Ordering:** subscribers called in registration order. Commutativity means this is convention, not requirement.

## Rust's Type System as Proof

The `T: 'static` bound ensures event types have no borrowed data — they are self-contained values. The `&E` reference in callbacks ensures immutability of the event during dispatch. The borrow checker ensures callback closures do not alias mutable state with each other (unless explicitly shared via `Rc<Cell<_>>` or similar, which is the caller's choice and responsibility).

Rust does not merely *permit* the commutativity argument — it *enforces* its preconditions.
