use crate::types::Address;
use crate::storage::StorageDoubleMap;
use crate::host::revert;

pub const DEFAULT_ADMIN_ROLE: [u8; 32] = [0u8; 32];

pub fn hash_role(role_name: &str) -> [u8; 32] {
    crate::host::sha256(role_name.as_bytes())
}

pub fn minter_role() -> [u8; 32] { hash_role("MINTER_ROLE") }
pub fn pauser_role() -> [u8; 32] { hash_role("PAUSER_ROLE") }
pub fn timelock_role() -> [u8; 32] { hash_role("TIMELOCK_ROLE") }
pub fn emergency_pauser_role() -> [u8; 32] { hash_role("EMERGENCY_PAUSER") }
pub fn oracle_manager_role() -> [u8; 32] { hash_role("ORACLE_MANAGER_ROLE") }

pub const ROLES_MAP: StorageDoubleMap<[u8; 32], Address, bool> = StorageDoubleMap::new(b"roles");

pub fn has_role(role: [u8; 32], account: Address) -> bool {
    ROLES_MAP.get_or_default(&role, &account)
}

pub fn grant_role(role: [u8; 32], account: Address) {
    ROLES_MAP.set(&role, &account, &true);
}

pub fn revoke_role(role: [u8; 32], account: Address) {
    ROLES_MAP.set(&role, &account, &false);
}

pub fn check_role(role: [u8; 32], account: Address) {
    if !has_role(role, account) {
        revert("AccessControl: sender missing role");
    }
}
