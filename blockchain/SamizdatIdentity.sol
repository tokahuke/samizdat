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
        require(bytes(entity).length != 0, "Entity cannot be empty");
        require(ttl > 15 * 60, "TTL must be greater than 15 minutes");

        // DNS-safety check on the identity. Mirrors
        // `samizdat_common::identity::check_servable_identity` (see
        // common/src/identity.rs). Any identity that fails here cannot be
        // hosted at `<identity>.localhost:<port>` and would be a
        // shadow/phishing-shaped subdomain.
        _validateIdentity(identity);

        // Do update:
        entry.entity = entity;
        entry.owner = msg.sender;
        entry.ttl = ttl;

        // Insert:
        SamizdatIdentityStorage(identityStorage).setIdentity(identity, entry);
    }

    // Reserved labels rejected as identities. Match the list in
    // `common/src/identity.rs`. Stored as keccak256 digests so the lookup
    // is one hash per candidate rather than a string compare loop.
    // The four type-marker words (`object`, `series`, `collection`,
    // `edition`) are reserved because the proxy's typed-subdomain
    // dispatch routes `<type>-<id>.<host>` to a content-typed origin;
    // a bare `<type>` identity would shadow that namespace.
    function _isReservedLabel(bytes32 h) private pure returns (bool) {
        return
            h == keccak256("localhost") ||
            h == keccak256("local") ||
            h == keccak256("arpa") ||
            h == keccak256("test") ||
            h == keccak256("example") ||
            h == keccak256("invalid") ||
            h == keccak256("localhost4") ||
            h == keccak256("localhost6") ||
            h == keccak256("object") ||
            h == keccak256("series") ||
            h == keccak256("collection") ||
            h == keccak256("edition");
    }

    // Rejects identities whose bytes start with `<type>-` where `<type>`
    // is one of the four content-type marker words and at least one more
    // byte follows the hyphen. These shapes collide with the typed
    // subdomain dispatch (`object-<hash>.<host>`, `series-<key>.<host>`,
    // `collection-<hash>.<host>`, `edition-<hash>.<host>`). Hand-coded
    // byte comparisons; Solidity has no regex.
    function _hasReservedTypePrefix(bytes memory b) private pure returns (bool) {
        uint256 n = b.length;
        // "object-X": needs at least 8 bytes.
        if (n >= 8 &&
            b[0] == 0x6F && b[1] == 0x62 && b[2] == 0x6A &&
            b[3] == 0x65 && b[4] == 0x63 && b[5] == 0x74 &&
            b[6] == 0x2D) {
            return true;
        }
        // "series-X": needs at least 8 bytes.
        if (n >= 8 &&
            b[0] == 0x73 && b[1] == 0x65 && b[2] == 0x72 &&
            b[3] == 0x69 && b[4] == 0x65 && b[5] == 0x73 &&
            b[6] == 0x2D) {
            return true;
        }
        // "collection-X": needs at least 12 bytes.
        if (n >= 12 &&
            b[0] == 0x63 && b[1] == 0x6F && b[2] == 0x6C &&
            b[3] == 0x6C && b[4] == 0x65 && b[5] == 0x63 &&
            b[6] == 0x74 && b[7] == 0x69 && b[8] == 0x6F &&
            b[9] == 0x6E && b[10] == 0x2D) {
            return true;
        }
        // "edition-X": needs at least 9 bytes.
        if (n >= 9 &&
            b[0] == 0x65 && b[1] == 0x64 && b[2] == 0x69 &&
            b[3] == 0x74 && b[4] == 0x69 && b[5] == 0x6F &&
            b[6] == 0x6E && b[7] == 0x2D) {
            return true;
        }
        return false;
    }

    // Refuses identities the node cannot serve at a `<identity>.localhost`
    // subdomain. Rules mirror `check_servable_identity` in
    // `common/src/identity.rs`:
    //   - 1..=63 ASCII bytes.
    //   - Alphabet [a-z0-9-].
    //   - No leading or trailing '-'.
    //   - Not in the reserved label set.
    //   - Not the `xn--` punycode prefix.
    //   - Not all digits (numeric host ambiguity).
    //   - Not a 52-char base32 key shape (a-z, 2-7 only); would shadow
    //     a series-key subdomain.
    //   - Not a `<type>-<rest>` shape where `<type>` is one of the four
    //     content-type marker words; would shadow typed subdomain dispatch.
    function _validateIdentity(string calldata identity) private pure {
        bytes memory b = bytes(identity);
        uint256 n = b.length;
        require(n >= 1, "Identity cannot be empty");
        require(n <= 63, "Identity is too long (max 63 bytes)");

        bool allDigits = true;
        bool keyShape = (n == 52);
        for (uint256 i = 0; i < n; i++) {
            bytes1 c = b[i];
            bool isDigit = c >= 0x30 && c <= 0x39;
            bool isLower = c >= 0x61 && c <= 0x7A;
            bool isHyphen = c == 0x2D;
            require(
                isDigit || isLower || isHyphen,
                "Identity must use only a-z, 0-9 and '-'"
            );
            if (!isDigit) {
                allDigits = false;
            }
            // base32 lowercase alphabet is [a-z2-7]: digits 0, 1, 8, 9 and
            // hyphen break the key-shape match.
            if (keyShape) {
                bool inBase32 =
                    (c >= 0x61 && c <= 0x7A) || (c >= 0x32 && c <= 0x37);
                if (!inBase32) {
                    keyShape = false;
                }
            }
        }

        require(b[0] != 0x2D, "Identity cannot start with '-'");
        require(b[n - 1] != 0x2D, "Identity cannot end with '-'");
        require(!allDigits, "Identity cannot be all digits");
        require(!keyShape, "Identity must not match the 52-char base32 key shape");

        // Reject `xn--` punycode prefix.
        if (n >= 4) {
            require(
                !(b[0] == 0x78 && b[1] == 0x6E && b[2] == 0x2D && b[3] == 0x2D),
                "Identity must not start with 'xn--'"
            );
        }

        require(!_isReservedLabel(keccak256(b)), "Identity is a reserved label");
        require(
            !_hasReservedTypePrefix(b),
            "Identity must not start with a content-type marker followed by '-'"
        );
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
