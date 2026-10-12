// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console2} from "forge-std/Script.sol";
import {BridgeVault} from "../src/BridgeVault.sol";

/// @notice Deploys a BridgeVault and whitelists destinations.
/// RELAYER=0x.. FEE_RECIPIENT=0x.. FEE_BPS=10 DESTINATIONS=8453,1399811149 \
///   forge script script/Deploy.s.sol --rpc-url $RPC_URL --account deployer --broadcast
contract Deploy is Script {
    function run() external returns (BridgeVault vault) {
        address relayer = vm.envAddress("RELAYER");
        address feeRecipient = vm.envAddress("FEE_RECIPIENT");
        uint16 feeBps = uint16(vm.envOr("FEE_BPS", uint256(10)));
        uint256[] memory dests = vm.envUint("DESTINATIONS", ",");

        vm.startBroadcast();
        vault = new BridgeVault(relayer, feeRecipient, feeBps);
        for (uint256 i; i < dests.length; ++i) {
            vault.setDestination(dests[i], true);
        }
        vm.stopBroadcast();

        console2.log("BridgeVault", address(vault));
    }
}
