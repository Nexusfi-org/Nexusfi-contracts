use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::json_types::U128;
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::store::IterableMap;
use near_sdk::{env, near_bindgen, AccountId, NearToken, PanicOnDefault, Promise};

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone)]
#[serde(crate = "near_sdk::serde")]
pub struct FundMetadata {
    pub name: String,
    pub symbol: String,
    pub description: Option<String>,
    pub assets: Vec<AssetInfo>,
}

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone)]
#[serde(crate = "near_sdk::serde")]
pub struct AssetInfo {
    pub name: String,
    pub contract_address: AccountId,
    pub weight: u8,
}

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone)]
#[serde(crate = "near_sdk::serde")]
pub struct Fund {
    pub metadata: FundMetadata,
    pub token_address: AccountId,
    pub total_supply: U128,
    pub creation_timestamp: u64,
}

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct IndexFundFactory {
    pub owner_id: AccountId,
    pub funds: IterableMap<String, Fund>,
    pub fund_creation_deposit: NearToken,
}

#[near_bindgen]
impl IndexFundFactory {
    #[init]
    pub fn new(owner_id: AccountId, fund_creation_deposit: NearToken) -> Self {
        Self {
            owner_id,
            funds: IterableMap::new(b"f"),
            fund_creation_deposit,
        }
    }

    #[payable]
    pub fn create_fund(&mut self, prefix: String, metadata: FundMetadata) -> Promise {
        // Validate deposit
        let deposit = env::attached_deposit();
        assert!(
            deposit >= self.fund_creation_deposit,
            "Insufficient deposit for fund creation"
        );

        // Validate total weight is 100%
        let total_weight: u8 = metadata.assets.iter().map(|a| a.weight).sum();
        assert_eq!(total_weight, 100, "Total weight must be 100%");

        // Generate unique subaccount name
        let subaccount_id = format!("{}.{}", prefix, env::current_account_id());

        // Create the fund token contract
        Promise::new(subaccount_id.parse().unwrap())
            .create_account()
            .transfer(deposit)
            .deploy_contract(include_bytes!("./wasm/token.wasm").to_vec())
            .function_call(
                "new".to_string(),
                near_sdk::serde_json::to_vec(&(env::predecessor_account_id(), metadata.assets))
                    .unwrap(),
                NearToken::from_near(0),
                near_sdk::Gas::from_tgas(100),
            )
    }

    pub fn get_fund(&self, prefix: String) -> Option<Fund> {
        self.funds.get(&prefix).cloned()
    }

    pub fn get_funds(&self, from_index: u64, limit: u64) -> Vec<(String, Fund)> {
        let keys: Vec<_> = self.funds.keys().collect(); // Collect references to keys
        let start: usize = from_index
            .try_into()
            .unwrap_or_else(|_| env::panic_str("Invalid from_index"));
        let end = std::cmp::min((from_index + limit) as usize, keys.len());
    
        keys[start..end]
            .iter()
            .map(|key| {
                (
                    (*key).clone(),                    // Dereference and clone the String
                    self.funds.get(*key).unwrap().clone(), // Dereference the key and clone the value
                )
            })
            .collect()
    }

    pub fn get_fund_creation_deposit(&self) -> NearToken {
        self.fund_creation_deposit
    }

    #[private]
    pub fn on_fund_created(
        &mut self,
        prefix: String,
        metadata: FundMetadata,
        token_address: AccountId,
    ) -> bool {
        let fund = Fund {
            metadata,
            token_address,
            total_supply: U128(0),
            creation_timestamp: env::block_timestamp(),
        };
        self.funds.insert(prefix, fund);
        true
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
            .attached_deposit( NearToken::from_near(10_000_000_000_000_000_000_000_000)) // 10 NEAR
            .build()
    }

    #[test]
    fn test_new() {
        let context = get_context(accounts(1));
        testing_env!(context);
        let contract = IndexFundFactory::new(
            accounts(1),
            NearToken::from_near(10_000_000_000_000_000_000_000_000), // 10 NEAR
        );
        assert_eq!(
            contract.get_fund_creation_deposit(),
            NearToken::from_near(10_000_000_000_000_000_000_000_000)
        );
    }

    #[test]
    #[should_panic(expected = "Total weight must be 100%")]
    fn test_create_fund_invalid_weights() {
        let context = get_context(accounts(1));
        testing_env!(context);
        let mut contract =
            IndexFundFactory::new(accounts(1), NearToken::from_near(10_000_000_000_000_000_000_000_000));

        let metadata = FundMetadata {
            name: "Test Fund".to_string(),
            symbol: "TEST".to_string(),
            description: Some("Test Description".to_string()),
            assets: vec![
                AssetInfo {
                    name: "ETH".to_string(),
                    contract_address: accounts(2),
                    weight: 30,
                },
                AssetInfo {
                    name: "BTC".to_string(),
                    contract_address: accounts(3),
                    weight: 30,
                },
            ],
        };

        contract.create_fund("test".to_string(), metadata);
    }
}
