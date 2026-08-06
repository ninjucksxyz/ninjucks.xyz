export {
  // Cosmos-native (Keplr) signing
  MsgExecuteContract, ChainRestAuthApi, createTransaction, TxRestApi, BaseAccount, TxClient,
  // EVM (MetaMask / Rabby / Brave) EIP-712 signing
  ChainRestTendermintApi, getEip712TypedData, createWeb3Extension, createTxRawEIP712,
  recoverTypedSignaturePubKey, hexToBase64, getInjectiveAddress, SIGN_EIP712,
  MsgExecuteContractCompat,   // wasm execute with msg-as-string, required for EIP-712
} from "@injectivelabs/sdk-ts";
