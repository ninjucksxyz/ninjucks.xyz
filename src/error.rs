use cosmwasm_std::{OverflowError, StdError, Uint128};
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Overflow(#[from] OverflowError),

    #[error("unauthorized")]
    Unauthorized,

    #[error("exactly one offer coin must be supplied as funds")]
    OfferCoinRequired,

    #[error("unknown venue")]
    UnknownVenue,

    #[error("slippage: received {received} < minimum_receive {minimum}")]
    SlippageExceeded { received: Uint128, minimum: Uint128 },

    #[error("minimum_receive must be greater than zero")]
    ZeroMinimumReceive,

    #[error("nothing received from the swap")]
    NothingReceived,

    #[error("no admin transfer is pending")]
    NoPendingAdmin,

    #[error("invalid injective_exec payload: {0}")]
    InvalidExecPayload(String),
}
