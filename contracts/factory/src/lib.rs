use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::json_types::U128;
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::store::IterableMap;
use near_sdk::{env, log, near_bindgen, AccountId, NearToken, PanicOnDefault, Promise, PublicKey, Gas, PromiseError};

const TGAS: Gas = Gas::from_tgas(1);
const NO_DEPOSIT: NearToken = NearToken::from_near(0); // 0 yⓃ
const NEAR_PER_STORAGE: NearToken = NearToken::from_yoctonear(10u128.pow(19)); // 10 NEAR
const DEFAULT_TOKEN_WASM: &[u8] = include_bytes!("./tokenf/token.wasm");

/// Metadata for an index fund
#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone)]
#[serde(crate = "near_sdk::serde")]
pub struct FundMetadata {
    pub name: String,
    pub symbol: String,
    pub description: Option<String>,
    pub assets: Vec<AssetInfo>,
}

/// Metadata for individual assets in an index fund
#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone)]
#[serde(crate = "near_sdk::serde")]
pub struct AssetInfo {
    pub name: String,
    pub contract_address: AccountId,
    pub weight: u8,
}

/// Information about a deployed fund
#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone)]
#[serde(crate = "near_sdk::serde")]
pub struct Fund {
    pub metadata: FundMetadata,
    pub token_address: AccountId,
    pub total_supply: U128,
    pub creation_timestamp: u64,
}

/// The IndexFundFactory contract
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

    /// Deploy a new fund as a subaccount
    #[payable]
    pub fn create_fund(
        &mut self,
        prefix: String,
        metadata: FundMetadata,
        public_key: Option<PublicKey>,
    ) -> Promise {
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

        // Calculate storage and code costs
        let contract_bytes = DEFAULT_TOKEN_WASM.len() as u128;
        let storage_cost = NEAR_PER_STORAGE.saturating_mul(contract_bytes);
        let minimum_needed = storage_cost.saturating_add(NearToken::from_millinear(100));
        
        assert!(deposit >= minimum_needed, "Attach at least {minimum_needed} yⓃ");

        let init_args = near_sdk::serde_json::to_vec(&(env::predecessor_account_id(), metadata.assets))
            .expect("Failed to serialize init args");

        // Deploy the fund token contract
        let mut promise = Promise::new(subaccount_id.parse().unwrap())
            .create_account()
            .transfer(deposit)
            .deploy_contract(DEFAULT_TOKEN_WASM.to_vec())
            .function_call("new".to_string(), init_args, NO_DEPOSIT, TGAS.saturating_mul(5));

        // Add full access key if provided
        if let Some(pk) = public_key {
            promise = promise.add_full_access_key(pk);
        }

        // Add callback
        promise.then(
            Self::ext(env::current_account_id()).on_fund_created_callback(
                prefix.clone(),
                metadata.clone(),
                subaccount_id,
            ),
        )
    }

    /// Callback to finalize fund creation
    #[private]
    pub fn on_fund_created_callback(
        &mut self,
        prefix: String,
        metadata: FundMetadata,
        token_address: AccountId,
        #[callback_result] result: Result<(), PromiseError>,
    ) -> bool {
        if let Ok(_) = result {
            let fund = Fund {
                metadata,
                token_address,
                total_supply: U128(0),
                creation_timestamp: env::block_timestamp(),
            };
            self.funds.insert(prefix, fund);
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
            .map(|key| {
                (
                    (*key).clone(),
                    self.funds.get(*key).unwrap().clone(),
                )
            })
            .collect()
    }
}
