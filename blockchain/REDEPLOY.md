# Redeploy debt: SamizdatIdentityV1

`blockchain/SamizdatIdentity.sol` has been tightened in source to refuse
identity registrations that no samizdat node can serve at a
`<identity>.localhost:<port>` subdomain. The validation rules mirror the
runtime helper at `common/src/identity.rs::check_servable_identity`.

- Reserved: added the four content-type marker words (`object`, `series`,
  `collection`, `edition`) to the reserved-label set and rejected any
  identity starting with `<type>-` for any of those four words; mirrors
  the typed-subdomain dispatch added on the proxy side.

**The live deployment is unchanged.** New registrations on-chain are still
accepted by the existing contract bytecode for any non-empty string not
starting with `_`. Until this contract is redeployed, the runtime checks in
the node, proxy, and CLI are the only defense against unservable names.
After redeploy, the runtime checks stay (they still bound the corpus of
names that were registered under the loose contract).

## When to redeploy

When you (the contract owner) want to. There is no functional regression
for existing users from leaving the live contract loose; the runtime
filters keep the node safe. The redeploy is purely forward-looking: it
prevents new garbage from being added on-chain.

## What the redeploy entails

1. Compile `SamizdatIdentity.sol` and deploy a fresh `SamizdatIdentityV1`
   against the EXISTING `SamizdatIdentityStorage` address. The storage
   contract is shared; we are only swapping the operator.
2. From the live V1, as `isOwner`, call `deprecate(<new-V1-address>)`.
   The `deprecate` flow in `SamizdatIdentity.sol` rotates the storage's
   operator to the new V1 via `SamizdatIdentityStorage::setOperator`.
3. Update the contract address constant the node reads. Check
   `common/src/blockchain.rs` for the address constant and the ABI loader;
   the ABI in `blockchain/SamizdatIdentityV1.json` is functionally
   identical to V2 because the public surface (`register`,
   `registerWithTtl`, `transfer`, `getIdentity`) did not change.
4. Push a new patch release with the updated address so installed nodes
   pick it up after `samizdat-up update`.

## What is NOT affected

- Existing identities in `SamizdatIdentityStorage::identities`. Storage
  rows are kept; only the registration-validation tightens going forward.
- `getIdentity` lookups for ANY name, including the pre-redeploy garbage.
  The CLI's `samizdat identity get <bad-name>` still works; only the
  hosting path at `<bad-name>.localhost:<port>` refuses to serve.
- The runtime `check_servable_identity` defense in the node, proxy, and
  CLI. These stay even after redeploy.

## Gas

The validation loop is bounded at 63 iterations. Reserved-label checks are
a constant number of keccak256 invocations against a 32-byte digest. The
extra gas cost on a `registerWithTtl` call is well under the existing
storage-write cost. No issue.
