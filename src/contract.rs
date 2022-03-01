#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::CosmosMsg::Bank;
use cosmwasm_std::{
    from_slice, to_binary, Addr, BankMsg, Binary, Coin, CosmosMsg, Deps, DepsMut, Env, MessageInfo,
    Order, OverflowError, Response, StdError, StdResult, Uint128, Uint64, WasmMsg,
};
use cw2::set_contract_version;
use cw20::Denom::Cw20;
use cw20::{Cw20Contract, Cw20ExecuteMsg, Cw20ReceiveMsg};
use cw_storage_plus::Bound;
use cw_utils::{NativeBalance, Scheduled};
use std::ops::{Add, Deref};

use crate::error::ContractError;
use crate::msg::{
    ExecuteMsg, InstantiateMsg, ListLockboxResponse, LockboxResponse, QueryMsg, ReceiveMsg,
};
use crate::state::{Claim, Config, Lockbox, CONFIG, LOCKBOXES, LOCK_BOX_SEQ};

// version info for migration info
const CONTRACT_NAME: &str = "crates.io:cw1-lockbox";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    _msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    let state = Config {};
    CONFIG.save(deps.storage, &state)?;

    LOCK_BOX_SEQ.save(deps.storage, &Uint64::zero())?;

    Ok(Response::new()
        .add_attribute("method", "instantiate")
        .add_attribute("owner", info.sender))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::CreateLockbox {
            owner,
            claims,
            expiration,
            native_token,
            cw20_addr,
        } => execute_create_lockbox(
            deps,
            env,
            info,
            owner,
            claims,
            expiration,
            native_token,
            cw20_addr,
        ),
        ExecuteMsg::Reset { id } => execute_reset(deps, env, info, id),
        ExecuteMsg::Deposit { id } => execute_deposit_native(deps, env, info, id),
        ExecuteMsg::Receive(msg) => execute_receive(deps, env, info, msg),
        ExecuteMsg::Claim { id } => execute_claim(deps, env, info, id),
    }
}

pub fn execute_create_lockbox(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    owner: String,
    claims: Vec<Claim>,
    expiration: Scheduled,
    native_token: Option<String>,
    cw20_addr: Option<String>,
) -> Result<Response, ContractError> {
    let owner = deps.api.addr_validate(&owner)?;

    if expiration.is_triggered(&env.block) {
        return Err(ContractError::LockboxExpired {});
    }

    match (native_token.clone(), cw20_addr) {
        (Some(_), Some(_)) => Err(ContractError::NotSupportDenom {}),
        (None, None) => Err(ContractError::NotSupportDenom {}),
        (_, _) => Ok(()),
    }?;

    let total_amount: Uint128 = claims.clone().into_iter().map(|c| c.amount).sum();

    let id = LOCK_BOX_SEQ.update::<_, StdError>(deps.storage, |id| Ok(id.add(Uint64::new(1))))?;

    let lockbox = Lockbox {
        id,
        owner,
        claims,
        expiration,
        total_amount,
        reset: false,
        native_denom: native_token,
        cw20_addr: None,
    };

    LOCKBOXES.save(deps.storage, id.u64(), &lockbox)?;

    Ok(Response::new().add_attribute("method", "execute_create_lockbox"))
}

pub fn execute_deposit_native(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    id: Uint64,
) -> Result<Response, ContractError> {
    let mut lockbox = LOCKBOXES.load(deps.storage, id.u64())?;
    if lockbox.expiration.is_triggered(&env.block) {
        return Err(ContractError::LockboxExpired {});
    }

    let denom = lockbox
        .native_denom
        .clone()
        .ok_or(ContractError::SendNativeTokens {})?;

    let coin: &Coin = info
        .funds
        .iter()
        .find(|c| c.denom == denom)
        .ok_or(ContractError::NotSupportDenom {})?;

    lockbox.total_amount = lockbox.total_amount.checked_sub(coin.amount)?;
    LOCKBOXES.save(deps.storage, id.u64(), &lockbox)?;

    Ok(Response::default()
        .add_attribute("action", "deposit")
        .add_attribute("amount", coin.amount))
}

pub fn execute_receive(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    wrapper: Cw20ReceiveMsg,
) -> Result<Response, ContractError> {
    let msg: ReceiveMsg = from_slice(&wrapper.msg)?;
    let amount = wrapper.amount;
    match msg {
        ReceiveMsg::Deposit { id } => execute_deposit(deps, env, info, id, amount),
    }
}

pub fn execute_deposit(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    id: Uint64,
    amount: Uint128,
) -> Result<Response, ContractError> {
    let mut lockbox = LOCKBOXES.load(deps.storage, id.u64())?;
    let cw20_addr = lockbox
        .clone()
        .cw20_addr
        .ok_or(ContractError::NotSupportDenom {})?;
    if info.sender != cw20_addr {
        return Err(ContractError::Unauthorized {});
    }

    lockbox.total_amount = lockbox.total_amount.checked_sub(amount)?;
    LOCKBOXES.save(deps.storage, id.u64(), &lockbox)?;

    Ok(Response::default()
        .add_attribute("action", "deposit")
        .add_attribute("amount", amount))
}

pub fn execute_claim(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    id: Uint64,
) -> Result<Response, ContractError> {
    let lockbox = LOCKBOXES.load(deps.storage, id.u64())?;

    if lockbox.reset {
        return Err(ContractError::Reset {});
    }
    if !lockbox.expiration.is_triggered(&env.block) {
        return Err(ContractError::LockboxUnexpired {});
    }

    let claim = lockbox
        .claims
        .into_iter()
        .find(|c| c.addr == info.sender.to_string())
        .ok_or(ContractError::Unauthorized {})?;

    let msg: CosmosMsg = match (lockbox.cw20_addr, lockbox.native_denom) {
        (None, None) => Err(ContractError::Unauthorized {}),
        (Some(_), Some(_)) => Err(ContractError::Unauthorized {}),
        (Some(cw20_addr), None) => {
            let msg = Cw20ExecuteMsg::Transfer {
                recipient: claim.addr.to_string(),
                amount: claim.amount,
            };
            Cw20Contract(cw20_addr)
                .call(msg)
                .map_err(ContractError::Std)
        }
        (None, Some(native)) => {
            let balance = deps
                .querier
                .query_balance(env.contract.address, native.clone())?;
            if balance.amount < claim.amount {
                return Err(ContractError::InsufficientBalance {});
            }
            let msg = BankMsg::Send {
                to_address: claim.addr.to_string(),
                amount: vec![Coin {
                    denom: native,
                    amount: claim.amount,
                }],
            };
            Ok(CosmosMsg::Bank(msg))
        }
    }?;

    let res = Response::new().add_message(msg);
    Ok(res)
}

pub fn execute_reset(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    id: Uint64,
) -> Result<Response, ContractError> {
    let mut lockbox = LOCKBOXES.load(deps.storage, id.u64())?;
    if info.sender != lockbox.owner {
        return Err(ContractError::Unauthorized {});
    }

    lockbox.reset = true;
    LOCKBOXES.save(deps.storage, id.u64(), &lockbox)?;

    Ok(Response::new()
        .add_attribute("action", "reset")
        .add_attribute("id", id))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::GetLockBox { id } => to_binary(&query_lockbox(deps, id)?),
        QueryMsg::ListLockBox { start_after, limit } => {
            to_binary(&range_lockbox(deps, start_after, limit)?)
        }
    }
}

fn query_lockbox(deps: Deps, id: Uint64) -> StdResult<LockboxResponse> {
    let lockbox = LOCKBOXES.load(deps.storage, id.u64())?;
    let res = LockboxResponse {
        id,
        owner: lockbox.owner,
        claims: lockbox.claims,
        expiration: lockbox.expiration,
        total_amount: lockbox.total_amount,
        resetted: lockbox.reset,
    };
    Ok(res)
}

// settings for pagination
const MAX_LIMIT: u32 = 30;
const DEFAULT_LIMIT: u32 = 10;

fn range_lockbox(
    deps: Deps,
    start_after: Option<u64>,
    limit: Option<u32>,
) -> StdResult<ListLockboxResponse> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let start = start_after.map(Bound::exclusive);
    let lockboxes: StdResult<Vec<_>> = LOCKBOXES
        .range(deps.storage, start, None, Order::Ascending)
        .take(limit)
        .collect();

    let res = ListLockboxResponse {
        lockboxes: lockboxes?.into_iter().map(|l| l.1.into()).collect(),
    };
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{
        mock_dependencies, mock_dependencies_with_balance, mock_env, mock_info,
    };
    use cosmwasm_std::{coins, from_binary};

    /*
    #[test]
    fn proper_initialization() {
        let mut deps = mock_dependencies_with_balance(&coins(2, "token"));

        let msg = InstantiateMsg { count: 17 };
        let info = mock_info("creator", &coins(1000, "earth"));

        // we can just call .unwrap() to assert this was a success
        let res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // it worked, let's query the state
        let res = query(deps.as_ref(), mock_env(), QueryMsg::GetCount {}).unwrap();
        let value: CountResponse = from_binary(&res).unwrap();
        assert_eq!(17, value.count);
    }

    #[test]
    fn reset() {
        let mut deps = mock_dependencies_with_balance(&coins(2, "token"));

        let msg = InstantiateMsg { count: 17 };
        let info = mock_info("creator", &coins(2, "token"));
        let _res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

        // beneficiary can release it
        let unauth_info = mock_info("anyone", &coins(2, "token"));
        let msg = ExecuteMsg::Reset { count: 5 };
        let res = execute(deps.as_mut(), mock_env(), unauth_info, msg);
        match res {
            Err(ContractError::Unauthorized {}) => {}
            _ => panic!("Must return unauthorized error"),
        }

        // only the original creator can reset the counter
        let auth_info = mock_info("creator", &coins(2, "token"));
        let msg = ExecuteMsg::Reset { count: 5 };
        let _res = execute(deps.as_mut(), mock_env(), auth_info, msg).unwrap();

        // should now be 5
        let res = query(deps.as_ref(), mock_env(), QueryMsg::GetCount {}).unwrap();
        let value: CountResponse = from_binary(&res).unwrap();
        assert_eq!(5, value.count);
    }

     */

    #[test]
    fn create_lockbox() {
        let mut deps = mock_dependencies();

        let msg = InstantiateMsg {
            admin: "ADMIN".to_string(),
        };
        let info = mock_info("creator", &[]);
        let mut env = mock_env();
        env.block.height = 1;
        let _res = instantiate(deps.as_mut(), env, info.clone(), msg).unwrap();

        let claims = vec![
            Claim {
                addr: Addr::unchecked("claim1"),
                amount: Uint128::new(4),
            },
            Claim {
                addr: Addr::unchecked("claim2".to_string()),
                amount: Uint128::new(15),
            },
        ];
        let msg = ExecuteMsg::CreateLockbox {
            owner: "OWNER".to_string(),
            claims: claims.clone(),
            expiration: Scheduled::AtHeight(5),
            native_token: Some("test".to_string()),
            cw20_addr: None,
        };
        let err = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap_err();
        assert_eq!(err, ContractError::LockboxExpired {});

        let msg = ExecuteMsg::CreateLockbox {
            owner: "OWNER".to_string(),
            claims: claims.clone(),
            expiration: Scheduled::AtHeight(100_000),
            native_token: Some("test".to_string()),
            cw20_addr: None,
        };
        let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

        let res = query_lockbox(deps.as_ref(), Uint64::new(1)).unwrap();
        assert_eq!(res.id, Uint64::new(1))
    }
}
