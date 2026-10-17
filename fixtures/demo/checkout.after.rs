use crate::payments::{authorize, capture};

pub fn checkout(cart: Cart) -> Result<Receipt, Error> {
    if cart.is_empty() {
        return Err(Error::EmptyCart);
    }
    let total = cart.total();
    let hold = authorize(total)?;
    inventory.reserve(&cart)?;
    let receipt = capture(hold)?;
    events.publish(OrderPlaced::new(&receipt));
    return Ok(receipt);
}
