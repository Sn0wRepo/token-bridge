// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {BridgeVault} from "../src/BridgeVault.sol";
import {BridgedToken} from "../src/BridgedToken.sol";
import {MockERC20} from "./mocks/MockERC20.sol";

/// @dev Fee recipient that tries to re-enter the vault while receiving ETH.
contract ReentrantFeeSink {
    BridgeVault public vault;

    function setVault(BridgeVault v) external {
        vault = v;
    }

    receive() external payable {
        vault.bridgeNative{value: msg.value}(8453, bytes32(uint256(1)));
    }
}

contract BridgeVaultTest is Test {
    uint256 internal constant BASE = 8453;
    uint256 internal constant SOLANA = 1_399_811_149;
    bytes32 internal constant SOL_KEY = 0x7f2f8f1f5a7b1d0e3bf3c3f8f1d9e5b2a0c4d6e8f0a1b2c3d4e5f60718293a4b;

    address internal relayer = makeAddr("relayer");
    address internal feeTo = makeAddr("feeTo");
    address internal alice = makeAddr("alice");
    address internal bob = makeAddr("bob");

    BridgeVault internal vault;

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

    function setUp() public {
        vm.chainId(1);
        vault = new BridgeVault(relayer, feeTo, 10); // 0.10%
        vault.setDestination(BASE, true);
        vault.setDestination(SOLANA, true);
        vm.deal(alice, 100 ether);
    }

    function _r(address a) internal pure returns (bytes32) {
        return bytes32(uint256(uint160(a)));
    }

    // ------------------------------------------------------------ outbound native

    function test_BridgeNativeEmitsAndTakesFee() public {
        vm.expectEmit(true, true, true, true);
        emit BridgeRequested(1, alice, address(0), 0.999 ether, 0.001 ether, SOLANA, SOL_KEY);
        vm.prank(alice);
        vault.bridgeNative{value: 1 ether}(SOLANA, SOL_KEY);

        assertEq(feeTo.balance, 0.001 ether);
        assertEq(address(vault).balance, 0.999 ether);
        assertEq(vault.nonce(), 1);
    }

    function test_NonceIncrements() public {
        vm.startPrank(alice);
        vault.bridgeNative{value: 1 ether}(BASE, _r(alice));
        vault.bridgeNative{value: 1 ether}(BASE, _r(alice));
        vm.stopPrank();
        assertEq(vault.nonce(), 2);
    }

    function test_RevertWhen_DestinationUnsupported() public {
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(BridgeVault.UnsupportedDestination.selector, 10));
        vault.bridgeNative{value: 1 ether}(10, _r(alice));
    }

    function test_RevertWhen_DestinationIsSameChain() public {
        vault.setDestination(1, true);
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(BridgeVault.UnsupportedDestination.selector, 1));
        vault.bridgeNative{value: 1 ether}(1, _r(alice));
    }

    function test_RevertWhen_ZeroAmountOrRecipient() public {
        vm.startPrank(alice);
        vm.expectRevert(BridgeVault.ZeroAmount.selector);
        vault.bridgeNative(BASE, _r(alice));
        vm.expectRevert(BridgeVault.BadRecipient.selector);
        vault.bridgeNative{value: 1}(BASE, bytes32(0));
        vm.stopPrank();
    }

    function test_RevertWhen_FeeSinkReenters() public {
        ReentrantFeeSink sink = new ReentrantFeeSink();
        BridgeVault v = new BridgeVault(relayer, address(sink), 100);
        v.setDestination(BASE, true);
        sink.setVault(v);
        vm.prank(alice);
        vm.expectRevert(BridgeVault.TransferFailed.selector);
        v.bridgeNative{value: 1 ether}(BASE, _r(alice));
    }

    // ------------------------------------------------------------ outbound ERC-20

    function test_BridgeERC20LocksTokens() public {
        MockERC20 usdt = new MockERC20(false); // no return value, like USDT
        vault.setToken(address(usdt), true, false);
        usdt.mint(alice, 1000e6);

        vm.startPrank(alice);
        usdt.approve(address(vault), 1000e6);
        vault.bridgeERC20(address(usdt), 1000e6, BASE, _r(bob));
        vm.stopPrank();

        assertEq(usdt.balanceOf(address(vault)), 999e6);
        assertEq(usdt.balanceOf(feeTo), 1e6);
        assertEq(usdt.balanceOf(alice), 0);
    }

    function test_RevertWhen_TokenNotSupported() public {
        MockERC20 t = new MockERC20(true);
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(BridgeVault.UnsupportedToken.selector, address(t)));
        vault.bridgeERC20(address(t), 1, BASE, _r(bob));
    }

    function test_WrappedTokenRoundTrip() public {
        BridgedToken w = new BridgedToken("Wrapped Token", "wTKN", 18, address(vault));
        vault.setToken(address(w), true, true);

        vm.prank(relayer);
        vault.release(BASE, 1, address(w), alice, 100 ether);
        assertEq(w.balanceOf(alice), 100 ether);

        vm.startPrank(alice);
        w.approve(address(vault), 100 ether);
        vault.bridgeERC20(address(w), 100 ether, BASE, _r(alice));
        vm.stopPrank();

        assertEq(w.balanceOf(alice), 0);
        assertEq(w.balanceOf(feeTo), 0.1 ether);
        assertEq(w.totalSupply(), 0.1 ether, "net amount burned, fee kept as tokens");
    }

    // ------------------------------------------------------------ inbound

    function test_ReleaseNative() public {
        vm.deal(address(vault), 5 ether);
        vm.expectEmit(true, true, true, true);
        emit Released(BASE, 7, address(0), bob, 2 ether);
        vm.prank(relayer);
        vault.release(BASE, 7, address(0), bob, 2 ether);
        assertEq(bob.balance, 2 ether);
        assertTrue(vault.processed(vault.transferId(BASE, 7)));
    }

    function test_ReleaseLockedERC20() public {
        MockERC20 t = new MockERC20(true);
        vault.setToken(address(t), true, false);
        t.mint(address(vault), 50);
        vm.prank(relayer);
        vault.release(BASE, 1, address(t), bob, 50);
        assertEq(t.balanceOf(bob), 50);
    }

    function test_RevertWhen_Replayed() public {
        vm.deal(address(vault), 5 ether);
        vm.startPrank(relayer);
        vault.release(BASE, 7, address(0), bob, 1 ether);
        vm.expectRevert(BridgeVault.AlreadyProcessed.selector);
        vault.release(BASE, 7, address(0), bob, 1 ether);
        vm.stopPrank();
    }

    function test_RevertWhen_NotRelayer() public {
        vm.prank(alice);
        vm.expectRevert(BridgeVault.NotRelayer.selector);
        vault.release(BASE, 1, address(0), alice, 1);
    }

    // ------------------------------------------------------------ admin

    function test_PauseBlocksBothDirections() public {
        vault.setPaused(true);
        vm.prank(alice);
        vm.expectRevert(BridgeVault.IsPaused.selector);
        vault.bridgeNative{value: 1}(BASE, _r(alice));
        vm.prank(relayer);
        vm.expectRevert(BridgeVault.IsPaused.selector);
        vault.release(BASE, 1, address(0), alice, 1);
    }

    function test_FeeCap() public {
        vm.expectRevert(BridgeVault.FeeTooHigh.selector);
        vault.setFee(101, feeTo);
        vault.setFee(100, feeTo);
        assertEq(vault.feeBps(), 100);
    }

    function test_OnlyOwner() public {
        vm.startPrank(alice);
        vm.expectRevert(BridgeVault.NotOwner.selector);
        vault.setDestination(10, true);
        vm.expectRevert(BridgeVault.NotOwner.selector);
        vault.setRelayer(alice);
        vm.expectRevert(BridgeVault.NotOwner.selector);
        vault.setPaused(true);
        vm.expectRevert(BridgeVault.NotOwner.selector);
        vault.transferOwnership(alice);
        vm.stopPrank();
    }

    function test_BridgedTokenOnlyBridgeMints() public {
        BridgedToken w = new BridgedToken("W", "W", 18, address(vault));
        vm.expectRevert(BridgedToken.NotBridge.selector);
        w.mint(alice, 1);
    }

    // ------------------------------------------------------------ fuzz

    function testFuzz_QuoteSplitsExactly(uint256 amount, uint16 bps) public {
        bps = uint16(bound(bps, 0, 100));
        amount = bound(amount, 0, type(uint256).max / 100);
        vault.setFee(bps, feeTo);
        (uint256 net, uint256 fee) = vault.quote(amount);
        assertEq(net + fee, amount);
        assertLe(fee, amount / 100);
    }

    function testFuzz_ReleaseIdsAreUnique(uint256 src, uint256 n1, uint256 n2) public view {
        vm.assume(n1 != n2);
        assertTrue(vault.transferId(src, n1) != vault.transferId(src, n2));
    }
}
