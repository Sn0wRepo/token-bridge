// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @dev Plain ERC-20 for tests. With returnsBool = false it mimics tokens like USDT that return nothing.
contract MockERC20 {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    bool public immutable returnsBool;

    constructor(bool returnsBool_) {
        returnsBool = returnsBool_;
    }

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }

    function transfer(address to, uint256 amount) external {
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        _ret();
    }

    function transferFrom(address from, address to, uint256 amount) external {
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        _ret();
    }

    function _ret() private view {
        if (returnsBool) {
            assembly {
                mstore(0, 1)
                return(0, 32)
            }
        }
    }
}
