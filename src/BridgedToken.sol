// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title BridgedToken (PLACEHOLDER)
/// @notice Minimal ERC-20 representing a token that lives natively on another chain.
///         Only the BridgeVault on this chain can mint (inbound) and burn (outbound).
/// @dev    Dependency-free on purpose so it drops into any repo. Swap for OpenZeppelin ERC20
///         (+ ERC20Permit) when hardening.
contract BridgedToken {
    string public name;
    string public symbol;
    uint8 public immutable decimals;
    uint256 public totalSupply;

    address public bridge;
    address public owner;

    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);
    event BridgeSet(address indexed bridge);

    error NotBridge();
    error NotOwner();
    error InsufficientBalance();
    error InsufficientAllowance();
    error ZeroAddress();

    constructor(string memory name_, string memory symbol_, uint8 decimals_, address bridge_) {
        if (bridge_ == address(0)) revert ZeroAddress();
        name = name_;
        symbol = symbol_;
        decimals = decimals_;
        bridge = bridge_;
        owner = msg.sender;
        emit BridgeSet(bridge_);
    }

    // ----------------------------------------------------------------- ERC-20

    function transfer(address to, uint256 amount) external returns (bool) {
        _transfer(msg.sender, to, amount);
        return true;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        emit Approval(msg.sender, spender, amount);
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        _spendAllowance(from, msg.sender, amount);
        _transfer(from, to, amount);
        return true;
    }

    // ----------------------------------------------------------------- bridge

    function mint(address to, uint256 amount) external {
        if (msg.sender != bridge) revert NotBridge();
        if (to == address(0)) revert ZeroAddress();
        totalSupply += amount;
        unchecked {
            balanceOf[to] += amount;
        }
        emit Transfer(address(0), to, amount);
    }

    /// @notice Burn `amount` from `from`. The vault needs an allowance, same as a normal transferFrom.
    function burnFrom(address from, uint256 amount) external {
        if (msg.sender != bridge) revert NotBridge();
        _spendAllowance(from, msg.sender, amount);
        uint256 bal = balanceOf[from];
        if (bal < amount) revert InsufficientBalance();
        unchecked {
            balanceOf[from] = bal - amount;
            totalSupply -= amount;
        }
        emit Transfer(from, address(0), amount);
    }

    function setBridge(address bridge_) external {
        if (msg.sender != owner) revert NotOwner();
        if (bridge_ == address(0)) revert ZeroAddress();
        bridge = bridge_;
        emit BridgeSet(bridge_);
    }

    // --------------------------------------------------------------- internal

    function _transfer(address from, address to, uint256 amount) private {
        if (to == address(0)) revert ZeroAddress();
        uint256 bal = balanceOf[from];
        if (bal < amount) revert InsufficientBalance();
        unchecked {
            balanceOf[from] = bal - amount;
            balanceOf[to] += amount;
        }
        emit Transfer(from, to, amount);
    }

    function _spendAllowance(address from, address spender, uint256 amount) private {
        uint256 current = allowance[from][spender];
        if (current != type(uint256).max) {
            if (current < amount) revert InsufficientAllowance();
            unchecked {
                allowance[from][spender] = current - amount;
            }
        }
    }
}
