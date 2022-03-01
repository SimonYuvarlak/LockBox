/// Creates a lockbox.

///For creating JSON Shema which will be used with CosmJS
use schemars::JsonSchema;

///Necessaryimports
use serde::{Deserialize, Serialize};

use cosmwasm_std::{Addr, Uint128, Uint64};
use cw_storage_plus::{Item, Map};
use cw_utils::Scheduled;

/// Item stores one typed item at the given key.
/// This is an analog of Singleton.
/// It functions just as Path but doesn't use a Vec and thus has a const fn constructor.
///
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {}

pub const CONFIG: Item<Config> = Item::new("admin");

///
pub const LOCK_BOX_SEQ: Item<Uint64> = Item::new("lockbox_seq");

///Defining lockbox
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Lockbox {
    pub id: Uint64,
    /// Owner is the owner of lockbox
    pub owner: Addr,
    /// Users which can claim reward from lb
    pub claims: Vec<Claim>,
    /// The exparition date
    pub expiration: Scheduled,
    /// Total amount put in the lb
    pub total_amount: Uint128,
    /// Whether lb is resetted or not
    pub reset: bool,
    /// Native currency
    pub native_denom: Option<String>,
    /// Other cw20 tokens that can be used interchangeably
    pub cw20_addr: Option<Addr>,
}

/// Custom object for lb struct field
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Claim {
    pub addr: Addr,
    pub amount: Uint128,
}

/// Lockboxes
pub const LOCKBOXES: Map<u64, Lockbox> = Map::new("lock_boxes");
