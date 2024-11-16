use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::json_types::U128;
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::store::Vector;
use near_sdk::{env, near_bindgen, AccountId, PanicOnDefault};

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone)]
#[serde(crate = "near_sdk::serde")]
pub struct AssetInfo {
    pub name: String,
    pub contract_address: AccountId,
    pub weight: u8,
}

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct IndexFundToken {
    pub total_assets: U128,
    pub assets: Vector<AssetInfo>,
    pub owner_id: AccountId,
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct AssetArgs {
    name: String,
    contract_address: AccountId,
    weight: u8,
}

#[near_bindgen]
impl IndexFundToken {
    #[init]
    pub fn new(owner_id: AccountId) -> Self {
        assert!(!env::state_exists(), "Contract is already initialized");
        Self {
            total_assets: U128(0),
            assets: Vector::new(b"a"),
            owner_id,
        }
    }

    #[payable]
    pub fn add_asset(&mut self, asset: AssetArgs) {
        self.assert_owner();
        assert!(
            asset.weight > 0 && asset.weight <= 100,
            "Weight must be between 1 and 100"
        );

        let asset_name = asset.name.clone(); // Clone just the name for logging
        let new_asset = AssetInfo {
            name: asset.name,
            contract_address: asset.contract_address,
            weight: asset.weight,
        };

        self.assets.push(new_asset);
        self.total_assets = U128(self.total_assets.0 + asset.weight as u128);

        env::log_str(&format!("Added new asset: {}", asset_name));
    }

    pub fn remove_asset(&mut self, index: u64) {
        self.assert_owner();
        let index_u32: u32 = index.try_into().unwrap_or_else(|_| {
            env::panic_str("Index is too large");
        });
        assert!(index_u32 < self.assets.len(), "Invalid asset index");
        
        let asset = self.assets.get(index_u32).unwrap();
        let asset_name = asset.name.clone();
        let asset_weight = asset.weight;
        
        self.total_assets = U128(self.total_assets.0 - asset_weight as u128);
        self.assets.swap_remove(index_u32);
        
        env::log_str(&format!("Removed asset: {}", asset_name));
    }

    pub fn get_asset_info(&self, index: u64) -> Option<AssetInfo> {
        let index_u32: u32 = index.try_into().unwrap_or_else(|_| {
            env::panic_str("Index is too large");
        });
        if index_u32 < self.assets.len() {
            self.assets.get(index_u32).cloned()
        } else {
            None
        }
    }

    pub fn get_assets(&self, from_index: u64, limit: u64) -> Vec<AssetInfo> {
        let start: u32 = from_index.try_into().unwrap_or_else(|_| {
            env::panic_str("From index is too large");
        });
        let limit_u32: u32 = limit.try_into().unwrap_or_else(|_| {
            env::panic_str("Limit is too large");
        });
        
        let end = std::cmp::min(
            start.saturating_add(limit_u32),
            self.assets.len()
        );
        
        (start..end)
            .filter_map(|index| self.assets.get(index).cloned())
            .collect()
    }

    pub fn get_total_assets(&self) -> U128 {
        self.total_assets
    }

   pub fn get_number_of_assets(&self) -> u64 {
        self.assets.len().into()
    }

    fn assert_owner(&self) {
        assert_eq!(
            env::predecessor_account_id(),
            self.owner_id,
            "Only the owner can call this method"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::test_utils::{accounts, VMContextBuilder};
    use near_sdk::{testing_env, VMContext};

    fn get_context(predecessor_account_id: AccountId) -> VMContext {
        VMContextBuilder::new()
            .predecessor_account_id(predecessor_account_id)
            .build()
    }

    #[test]
    fn test_new() {
        let context = get_context(accounts(1));
        testing_env!(context);
        let contract = IndexFundToken::new(accounts(1));
        assert_eq!(contract.get_number_of_assets(), 0);
        assert_eq!(contract.get_total_assets(), U128(0));
    }

    #[test]
    fn test_add_asset() {
        let mut context = get_context(accounts(1));
        testing_env!(context.clone());
        let mut contract = IndexFundToken::new(accounts(1));

        let asset = AssetArgs {
            name: "Test Token".to_string(),
            contract_address: accounts(2),
            weight: 50,
        };

        testing_env!(context);
        contract.add_asset(asset);
        assert_eq!(contract.get_number_of_assets(), 1);
        assert_eq!(contract.get_total_assets(), U128(50));
    }

    #[test]
    #[should_panic(expected = "Only the owner can call this method")]
    fn test_add_asset_not_owner() {
        let mut context = get_context(accounts(1));
        testing_env!(context.clone());
        let mut contract = IndexFundToken::new(accounts(1));

        let asset = AssetArgs {
            name: "Test Token".to_string(),
            contract_address: accounts(2),
            weight: 50,
        };

        testing_env!(VMContextBuilder::new()
            .predecessor_account_id(accounts(2))
            .build());
        contract.add_asset(asset);
    }
}
