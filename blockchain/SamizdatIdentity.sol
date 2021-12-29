// SPDX-License-Identifier: AGPL-3.0

pragma solidity >=0.7.0 <0.9.0;

// @title The Samizdat identity registry contract.
contract SamizdatIdentity {
    struct Entry {
        /// The value of the this identity.
        string entity;
        /// The owner of this identity.
        address owner;
        /// The recommended time to look up this value again.
        uint64 ttl;
    }

    // The owner of this contract.
    address public owner;
    // The registry of all identities.
    mapping (string => Entry) public identities;

    // A fresh-new association.
    event NewAssociation(string identity, string entity);
    // An association was updated.
    event UpdateAssociation(string identity, string oldEntity, string newEntity);
    // Ownership transfered.
    event Transfer(string identity, address from, address to);

    constructor() {
        owner = msg.sender;
    }

    // Register an assocation (or update an existing one).
    function registerWithTtl(string calldata identity, string calldata entity, uint64 ttl) public {
        Entry storage entry = identities[identity];

        require(
            entry.owner == address(0) || entry.owner == msg.sender,
            "Must be owner of the identity to control it"
        );
        require(bytes(identity).length != 0, "Identity cannot be empty");
        require(bytes(entity).length != 0, "Entity cannot be empty");
        require(ttl > 15 * 60, "TTL must be grater than 15 minutes");

        if (entry.owner == address(0)) {
            emit NewAssociation(identity, entity);
        } else if (
            keccak256(abi.encodePacked(entity)) != keccak256(abi.encodePacked(entry.entity))
        ) {
            emit UpdateAssociation(identity, entry.entity, entity);
        }

        // Do update:
        entry.entity = entity;
        entry.owner = msg.sender;
        entry.ttl = ttl;
    }

    // Register an assocation (or update an existing one) with a TTL of 1 hour.
    function register(string calldata identity, string calldata entity) public {
        return registerWithTtl(identity, entity, 3600);
    }

    // Transfer the ownership of an entity to someone else
    function transfer(string calldata identity, address to) public {
        Entry storage entry = identities[identity];
        
        require(entry.owner != address(0), "Identity does not exist");
        require(entry.owner == msg.sender, "Must be owner of the identity to control it");
        require(to != address(0), "Cannot transfer to zero-address");

        if (entry.owner != to) {
            emit Transfer(identity, entry.owner, to);
        }

        entry.owner = to;
    }
}
