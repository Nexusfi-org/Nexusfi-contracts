use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::json_types::U128;
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::store::IterableMap;
use near_sdk::{
    env, log, near_bindgen, AccountId, Gas, NearToken, PanicOnDefault, Promise, PromiseError,
    PublicKey,
};
use schemars::JsonSchema;

const TGAS: Gas = Gas::from_tgas(1);
const NO_DEPOSIT: NearToken = NearToken::from_near(0);
const DEFAULT_TOKEN_WASM: &[u8] = include_bytes!("./token/token.wasm");

#[derive(Serialize, Deserialize, JsonSchema, Clone)]
#[serde(crate = "near_sdk::serde")]
pub struct U128Json {
    value: u128,
}

// Conversion implementations
impl From<U128> for U128Json {
    fn from(u128_value: U128) -> Self {
        Self {
            value: u128_value.0,
        }
    }
}

impl From<U128Json> for U128 {
    fn from(wrapper: U128Json) -> Self {
        U128(wrapper.value)
    }
}

impl BorshSerialize for U128Json {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        BorshSerialize::serialize(&self.value, writer)
    }
}

impl BorshDeserialize for U128Json {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let value = BorshDeserialize::deserialize_reader(reader)?;
        Ok(Self { value })
    }
}

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone, JsonSchema)]
#[serde(crate = "near_sdk::serde")]
pub struct FundMetadata {
    pub name: String,
    pub symbol: String,
    pub description: Option<String>,
    pub assets: Vec<AssetInfo>,
}

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone, JsonSchema)]
#[serde(crate = "near_sdk::serde")]
pub struct AssetInfo {
    pub name: String,
    pub contract_address: String,
    pub weight: u8,
}

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone, JsonSchema)]
#[serde(crate = "near_sdk::serde")]
pub struct Fund {
    pub metadata: FundMetadata,
    pub token_address: String,
    pub total_supply: U128Json,
    pub creation_timestamp: u64,
}

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct IndexFundFactory {
    pub funds: IterableMap<String, Fund>,
}

#[near_bindgen]
impl IndexFundFactory {
    #[init]
    pub fn new() -> Self {
        Self {
            funds: IterableMap::new(b"f"),
        }
    }

    #[payable]
    pub fn create_fund(
        &mut self,
        prefix: String,
        metadata: FundMetadata,
        public_key: Option<PublicKey>,
    ) -> Promise {
        // Validate total weight is 100%
        let total_weight: u8 = metadata.assets.iter().map(|a| a.weight).sum();
        assert_eq!(total_weight, 100, "Total weight must be 100%");

        // Generate unique subaccount name
        let subaccount_id = format!("{}.{}", prefix, env::current_account_id());
        let subaccount = subaccount_id.parse::<AccountId>().unwrap();

        // Prepare init arguments for the token contract
        let init_args = near_sdk::serde_json::to_vec(&(
            env::predecessor_account_id(), // owner_id
            metadata.assets.clone(),       // assets
            "3e2210e1184b45b64c8a434c0a7e7b23cc04ea7eb7a6c3c32520d03d4afcb8af".parse::<AccountId>().unwrap(), // hardcoded USDC contract
        )).expect("Failed to serialize init args");

        let deposit = env::attached_deposit();
        
        // Deploy the fund token contract
        let mut promise = Promise::new(subaccount.clone())
            .create_account()
            .transfer(deposit)
            .deploy_contract(DEFAULT_TOKEN_WASM.to_vec())
            .function_call(
                "new".to_string(),
                init_args,
                NO_DEPOSIT,
                Gas::from_tgas(50),
            );

        // Add full access key if provided
        if let Some(pk) = public_key {
            promise = promise.add_full_access_key(pk);
        }

        // Add callback
        promise.then(
            Self::ext(env::current_account_id())
                .with_static_gas(Gas::from_tgas(10))
                .on_fund_created_callback(
                    prefix,
                    metadata,
                    subaccount.to_string(),
                )
        )
    }

    #[private]
    pub fn on_fund_created_callback(
        &mut self,
        prefix: String,
        metadata: FundMetadata,
        token_address: String,
        #[callback_result] result: Result<(), PromiseError>,
    ) -> bool {
        if result.is_ok() {
            let fund = Fund {
                metadata,
                token_address: token_address.clone(),
                total_supply: U128Json::from(U128(0)),
                creation_timestamp: env::block_timestamp(),
            };

            self.funds.insert(prefix.clone(), fund);
            log!("Successfully created fund at {}", token_address);

            true
        } else {
            log!("Failed to create fund. Refunding attached deposit.");
            Promise::new(env::predecessor_account_id()).transfer(env::attached_deposit());
            false
        }
    }

    pub fn get_fund(&self, prefix: String) -> Option<Fund> {
        self.funds.get(&prefix).cloned()
    }

    pub fn get_funds(&self, from_index: u64, limit: u64) -> Vec<(String, Fund)> {
        let keys: Vec<_> = self.funds.keys().collect();
        let start: usize = from_index
            .try_into()
            .unwrap_or_else(|_| env::panic_str("Invalid from_index"));
        let end = std::cmp::min((from_index + limit) as usize, keys.len());

        keys[start..end]
            .iter()
            .map(|key| ((*key).clone(), self.funds.get(*key).unwrap().clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::test_utils::{accounts, VMContextBuilder};
    use near_sdk::testing_env;

    fn get_context(predecessor: AccountId) -> VMContextBuilder {
        let mut builder = VMContextBuilder::new();
        builder.predecessor_account_id(predecessor);
        builder
    }
}