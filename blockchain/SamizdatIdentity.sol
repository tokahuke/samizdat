// SPDX-License-Identifier: AGPL-3.0
pragma solidity >=0.8.12 <0.9.0;

struct Entry {
    // The value of the this identity.
    string entity;
    // The owner of this identity.
    address owner;
    // The recommended time to look up this value again.
    uint64 ttl;
    // Reserved for future use:
    bytes extraData;
}

// @title The Samizdat identity storage.
// @dev implements the raw storage for identities in the Samizdat Network.
contract SamizdatIdentityStorage {
    // The owner of this contract.
    address public owner;
    // The smart contract allowed to operate on this storage.
    address public operator;
    // The registry of all identities.
    mapping (string => Entry) public identities;

    constructor() {
        owner = msg.sender;
    }

    event SetIdentity(string identity, Entry from, Entry to);
    // Privilege-relevant changes emit events so off-chain indexers can detect
    // rotations and trigger cache invalidations.
    event OperatorChanged(address indexed previousOperator, address indexed newOperator);
    event OwnerChanged(address indexed previousOwner, address indexed newOwner);

    // Only the operator of the storage can do this.
    modifier operatorOnly() {
        require(msg.sender == operator, "Only the operator contract can run this");
        _;
    }

    // Changes the operator of this contract. Either the storage `owner` or
    // the current `operator` (i.e. the live `SamizdatIdentityV1` calling
    // `deprecate(successor)`) may rotate. The owner path exists so a
    // compromised operator can be evicted; the operator path exists so
    // V1 -> V2 upgrade via `deprecate` works without the human owner
    // needing to be online. Rejects the zero address (would brick the
    // storage because `operatorOnly` would reject every caller forever).
    function setOperator(address newOperator) public {
        require(
            msg.sender == operator || msg.sender == owner,
            "Only the current operator or storage owner can rotate the operator"
        );
        require(newOperator != address(0), "Operator cannot be the zero address");
        emit OperatorChanged(operator, newOperator);
        operator = newOperator;
    }

    // Transfers storage ownership. Without this, a lost deployer key bricks
    // the storage forever (operator can be rotated but owner is permanent).
    function changeOwner(address newOwner) public {
        require(msg.sender == owner, "Only the contract owner can change owner");
        require(newOwner != address(0), "Owner cannot be the zero address");
        emit OwnerChanged(owner, newOwner);
        owner = newOwner;
    }

    // Method for getting identitied to another contract.
    function getIdentity(string calldata identity) public view operatorOnly returns (Entry memory) {
        return identities[identity];
    }

    // Method for setting identities from another contract.
    function setIdentity(string calldata identity, Entry memory entry) public operatorOnly {
        emit SetIdentity(identity, identities[identity], entry);
        identities[identity] = entry;
    }
}

// @title The Samizdat identity registry contract.
// @dev implements the registry for identities in the Samizdat Network.
contract SamizdatIdentityV1 {
    // The owner of this contract.
    address payable public owner;
    // The contract holding the data from identities.
    address identityStorage;
    // The price of an identity.
    uint public price;
    // Sets this contract as deprecated. No more identities can be added to it.
    bool public isDeprecated = false;
    // The contract tht superseeds this one.
    address public superseedingContract;

    constructor(address _identityStorage) {
        identityStorage = _identityStorage;
        owner = payable(msg.sender);
    }

    event OwnerChanged(address indexed previousOwner, address indexed newOwner);
    event PriceChanged(uint previousPrice, uint newPrice);
    event Deprecated(address indexed superseedingContract);
    event Withdrawn(address indexed to, uint amount);

    modifier isOwner() {
        require(msg.sender == owner, "Only the contract owner can run this");
        _;
    }

    modifier notDeprecated() {
        require(
            !isDeprecated,
            "Current contract was deprecated in favor of the address in the"
            "superseedingContract property"
        );
        _;
    }

    // Changes the owner of the smart contract.
    function changeOwner(address payable newOwner) public isOwner {
        require(newOwner != address(0), "Owner cannot be the zero address");
        emit OwnerChanged(owner, newOwner);
        owner = newOwner;
    }

    // Changes the price of an identity.
    function setPrice(uint newPrice) public isOwner {
        emit PriceChanged(price, newPrice);
        price = newPrice;
    }

    // Allows the owner to withdraw funds from the contract. Uses `.call`
    // rather than the legacy `.transfer` because `.transfer` forwards only
    // 2300 gas, which breaks recipients whose receive hook costs more
    // (multisigs, smart-contract wallets like Gnosis Safe). Reverts cleanly
    // on insufficient balance and propagates inner reverts.
    function withdraw(uint amount) public isOwner {
        require(amount <= address(this).balance, "Insufficient balance");
        (bool ok, ) = owner.call{value: amount}("");
        require(ok, "Withdraw transfer failed");
        emit Withdrawn(owner, amount);
    }

    // Deprecates this contract in favor of another one.
    function deprecate(address _superseedingContract) public isOwner {
        require(_superseedingContract != address(0), "Superseding contract cannot be zero");
        isDeprecated = true;
        superseedingContract = _superseedingContract;
        emit Deprecated(_superseedingContract);
        SamizdatIdentityStorage(identityStorage).setOperator(superseedingContract);
    }

    receive() external payable { }

    // Register an association (or update an existing one).
    function registerWithTtl(
        string calldata identity, 
        string calldata entity,
        uint64 ttl
    ) payable public notDeprecated {
        Entry memory entry = SamizdatIdentityStorage(identityStorage).getIdentity(identity);

        if (entry.ttl == 0) {
            require(msg.value == price, "Need to pay the identity price to have it registered");
        } else {
            require(msg.value == 0, "Cannot pay for registered entity");
        }

        require(
            entry.owner == address(0) || entry.owner == msg.sender,
            "Must be owner of the identity to control it"
        );
        require(bytes(identity).length != 0, "Identity cannot be empty");
        require(bytes(identity)[0] != "_", "Identity canot start with `_`");
        require(bytes(entity).length != 0, "Entity cannot be empty");
        require(ttl > 15 * 60, "TTL must be greater than 15 minutes");

        // Do update:
        entry.entity = entity;
        entry.owner = msg.sender;
        entry.ttl = ttl;

        // Insert:
        SamizdatIdentityStorage(identityStorage).setIdentity(identity, entry);
    }

    // Register an association (or update an existing one) with a TTL of 1 hour.
    function register(
        string calldata identity,
        string calldata entity
    ) payable public notDeprecated {
        return registerWithTtl(identity, entity, 3600);
    }

    // Transfer the ownership of an entity to someone else
    function transfer(string calldata identity, address to) public notDeprecated {
        Entry memory entry = SamizdatIdentityStorage(identityStorage).getIdentity(identity);
        
        require(entry.owner != address(0), "Identity does not exist");
        require(entry.owner == msg.sender, "Must be owner of the identity to control it");
        require(to != address(0), "Cannot transfer to zero-address");

        entry.owner = to;

        // Insert:
        SamizdatIdentityStorage(identityStorage).setIdentity(identity, entry);
    }
}
