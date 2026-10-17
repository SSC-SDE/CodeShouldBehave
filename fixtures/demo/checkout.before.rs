use crate::payments::charge;

pub fn checkout(cart: Cart) -> Result<Receipt, Error> {
    let total = cart.total();
    let receipt = charge(total)?;
    inventory.reserve(&cart)?;
    return Ok(receipt);
}
