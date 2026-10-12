// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

interface IERC20Minimal {
    function transfer(address to, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
}

interface IBridgedToken {
    function mint(address to, uint256 amount) external;
    function burnFrom(address from, uint256 amount) external;
}

/// @title BridgeVault (PLACEHOLDER)
/// @notice Lock-and-release / burn-and-mint bridge endpoint deployed on every EVM chain
///         (Ethereum mainnet, Base, ...). Outbound transfers emit `BridgeRequested`;
///         an off-chain relayer watches those events and calls `release` on the destination vault.
/// @dev    NOT PRODUCTION READY. Trust is a single relayer key. Before mainnet, swap the relayer
///         for a real messaging layer (OP Stack StandardBridge for Ethereum<->Base, Wormhole /
///         LayerZero / CCIP for Solana) or at least an M-of-N signer set, then audit.
///
///         Recipients are `bytes32` so the same event serves EVM (left-padded address) and
///         Solana (raw 32-byte public key) destinations.
contract BridgeVault {
    /// @notice Pseudo address used for the chain's native coin (ETH).
    address public constant NATIVE = address(0);
    uint16 public constant MAX_FEE_BPS = 100; // 1%

    address public owner;
    address public relayer;
    address public feeRecipient;
    uint16 public feeBps;
    bool public paused;

    /// @notice Outbound nonce, unique per source vault.
    uint256 public nonce;

    /// @notice Destination ids this vault may send to (EVM chain ids, plus an agreed id for Solana).
    mapping(uint256 => bool) public supportedDestination;

    /// @notice Tokens accepted on this chain. Wrapped tokens are burned/minted instead of locked.
    mapping(address => bool) public supportedToken;
    mapping(address => bool) public isWrappedToken;

    /// @notice Replay protection for inbound releases: keccak256(srcChainId, srcNonce) => done.
    mapping(bytes32 => bool) public processed;

    event BridgeRequested(
        uint256 indexed nonce,
        address indexed sender,
        address indexed token,
        uint256 amount,
        uint256 fee,
        uint256 dstChainId,
        bytes32 recipient
    );
    event Released(
        uint256 indexed srcChainId, uint256 indexed srcNonce, address indexed token, address to, uint256 amount
    );
    event DestinationSet(uint256 indexed chainId, bool supported);
    event TokenSet(address indexed token, bool supported, bool wrapped);
    event RelayerSet(address indexed relayer);
    event FeeSet(uint16 feeBps, address feeRecipient);
    event PausedSet(bool paused);
    event OwnershipTransferred(address indexed previousOwner, address indexed newOwner);

    error NotOwner();
    error NotRelayer();
    error IsPaused();
    error ZeroAddress();
    error ZeroAmount();
    error BadRecipient();
    error UnsupportedDestination(uint256 chainId);
    error UnsupportedToken(address token);
    error AlreadyProcessed();
    error FeeTooHigh();
    error TransferFailed();
    error Reentrancy();

    uint256 private _lock = 1;

    modifier onlyOwner() {
        if (msg.sender != owner) revert NotOwner();
        _;
    }

    modifier onlyRelayer() {
        if (msg.sender != relayer) revert NotRelayer();
        _;
    }

    modifier whenNotPaused() {
        if (paused) revert IsPaused();
        _;
    }

    modifier nonReentrant() {
        if (_lock != 1) revert Reentrancy();
        _lock = 2;
        _;
        _lock = 1;
    }

    constructor(address relayer_, address feeRecipient_, uint16 feeBps_) {
        if (relayer_ == address(0) || feeRecipient_ == address(0)) revert ZeroAddress();
        if (feeBps_ > MAX_FEE_BPS) revert FeeTooHigh();
        owner = msg.sender;
        relayer = relayer_;
        feeRecipient = feeRecipient_;
        feeBps = feeBps_;
        supportedToken[NATIVE] = true;
        emit OwnershipTransferred(address(0), msg.sender);
    }

    // --------------------------------------------------------------- outbound

    /// @notice Bridge the native coin. `msg.value` includes the protocol fee.
    function bridgeNative(uint256 dstChainId, bytes32 recipient) external payable whenNotPaused nonReentrant {
        (uint256 net, uint256 fee) = _checkAndSplit(NATIVE, msg.value, dstChainId, recipient);
        if (fee != 0) _sendNative(feeRecipient, fee);
        emit BridgeRequested(++nonce, msg.sender, NATIVE, net, fee, dstChainId, recipient);
    }

    /// @notice Bridge an ERC-20. Caller must `approve` this vault for `amount` first.
    function bridgeERC20(address token, uint256 amount, uint256 dstChainId, bytes32 recipient)
        external
        whenNotPaused
        nonReentrant
    {
        if (token == NATIVE) revert UnsupportedToken(token);
        (uint256 net, uint256 fee) = _checkAndSplit(token, amount, dstChainId, recipient);

        if (isWrappedToken[token]) {
            IBridgedToken(token).burnFrom(msg.sender, net);
            if (fee != 0) _pullERC20(token, msg.sender, feeRecipient, fee);
        } else {
            _pullERC20(token, msg.sender, address(this), net);
            if (fee != 0) _pullERC20(token, msg.sender, feeRecipient, fee);
        }
        emit BridgeRequested(++nonce, msg.sender, token, net, fee, dstChainId, recipient);
    }

    // ---------------------------------------------------------------- inbound

    /// @notice Called by the relayer once a `BridgeRequested` event is final on the source chain.
    /// @param token Local token to pay out (NATIVE, a locked ERC-20, or a wrapped token to mint).
    function release(uint256 srcChainId, uint256 srcNonce, address token, address to, uint256 amount)
        external
        onlyRelayer
        whenNotPaused
        nonReentrant
    {
        if (to == address(0)) revert ZeroAddress();
        if (amount == 0) revert ZeroAmount();
        if (!supportedToken[token]) revert UnsupportedToken(token);
        bytes32 id = keccak256(abi.encode(srcChainId, srcNonce));
        if (processed[id]) revert AlreadyProcessed();
        processed[id] = true;

        if (token == NATIVE) _sendNative(to, amount);
        else if (isWrappedToken[token]) IBridgedToken(token).mint(to, amount);
        else _pushERC20(token, to, amount);

        emit Released(srcChainId, srcNonce, token, to, amount);
    }

    // ------------------------------------------------------------------ views

    function quote(uint256 amount) public view returns (uint256 net, uint256 fee) {
        fee = (amount * feeBps) / 10_000;
        net = amount - fee;
    }

    function transferId(uint256 srcChainId, uint256 srcNonce) external pure returns (bytes32) {
        return keccak256(abi.encode(srcChainId, srcNonce));
    }

    /// @notice Helper: EVM address -> bytes32 recipient.
    function toRecipient(address account) external pure returns (bytes32) {
        return bytes32(uint256(uint160(account)));
    }

    // ------------------------------------------------------------------ admin

    function setDestination(uint256 chainId, bool supported) external onlyOwner {
        supportedDestination[chainId] = supported;
        emit DestinationSet(chainId, supported);
    }

    function setToken(address token, bool supported, bool wrapped) external onlyOwner {
        supportedToken[token] = supported;
        isWrappedToken[token] = wrapped;
        emit TokenSet(token, supported, wrapped);
    }

    function setRelayer(address relayer_) external onlyOwner {
        if (relayer_ == address(0)) revert ZeroAddress();
        relayer = relayer_;
        emit RelayerSet(relayer_);
    }

    function setFee(uint16 feeBps_, address feeRecipient_) external onlyOwner {
        if (feeBps_ > MAX_FEE_BPS) revert FeeTooHigh();
        if (feeRecipient_ == address(0)) revert ZeroAddress();
        feeBps = feeBps_;
        feeRecipient = feeRecipient_;
        emit FeeSet(feeBps_, feeRecipient_);
    }

    function setPaused(bool value) external onlyOwner {
        paused = value;
        emit PausedSet(value);
    }

    function transferOwnership(address newOwner) external onlyOwner {
        if (newOwner == address(0)) revert ZeroAddress();
        emit OwnershipTransferred(owner, newOwner);
        owner = newOwner;
    }

    /// @notice Native liquidity for releases on this chain (lock-and-release model).
    receive() external payable {}

    // --------------------------------------------------------------- internal

    function _checkAndSplit(address token, uint256 amount, uint256 dstChainId, bytes32 recipient)
        private
        view
        returns (uint256 net, uint256 fee)
    {
        if (amount == 0) revert ZeroAmount();
        if (recipient == bytes32(0)) revert BadRecipient();
        if (dstChainId == block.chainid || !supportedDestination[dstChainId]) {
            revert UnsupportedDestination(dstChainId);
        }
        if (!supportedToken[token]) revert UnsupportedToken(token);
        (net, fee) = quote(amount);
    }

    function _sendNative(address to, uint256 amount) private {
        (bool ok,) = to.call{value: amount}("");
        if (!ok) revert TransferFailed();
    }

    function _pullERC20(address token, address from, address to, uint256 amount) private {
        (bool ok, bytes memory data) = token.call(abi.encodeCall(IERC20Minimal.transferFrom, (from, to, amount)));
        if (!ok || (data.length != 0 && !abi.decode(data, (bool)))) revert TransferFailed();
    }

    function _pushERC20(address token, address to, uint256 amount) private {
        (bool ok, bytes memory data) = token.call(abi.encodeCall(IERC20Minimal.transfer, (to, amount)));
        if (!ok || (data.length != 0 && !abi.decode(data, (bool)))) revert TransferFailed();
    }
}
