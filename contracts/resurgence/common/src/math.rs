use crate::host::revert;

pub fn add(a: u128, b: u128) -> u128 {
    a.checked_add(b).unwrap_or_else(|| revert("SafeMath: addition overflow"))
}

pub fn sub(a: u128, b: u128) -> u128 {
    a.checked_sub(b).unwrap_or_else(|| revert("SafeMath: subtraction overflow"))
}

pub fn mul(a: u128, b: u128) -> u128 {
    a.checked_mul(b).unwrap_or_else(|| revert("SafeMath: multiplication overflow"))
}

pub fn div(a: u128, b: u128) -> u128 {
    a.checked_div(b).unwrap_or_else(|| revert("SafeMath: division by zero"))
}
