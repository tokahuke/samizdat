resource "digitalocean_domain" "hubfederation" {
  name = "hubfederation.com"
}

resource "digitalocean_record" "tesbed_ipv4" {
  domain = digitalocean_domain.hubfederation.id
  type   = "A"
  name   = "testbed"
  value  = digitalocean_droplet.samizdat_testbed.ipv4_address
}

resource "digitalocean_record" "tesbed_ipv6" {
  domain = digitalocean_domain.hubfederation.id
  type   = "AAAA"
  name   = "testbed"
  value  = digitalocean_droplet.samizdat_testbed.ipv6_address
}

# Apex (`hubfederation.com`) and one-label-deep wildcard
# (`*.hubfederation.com`) both point at the testbed droplet. The proxy
# holds the apex + wildcard SANs on one Let's Encrypt cert via ACME
# DNS-01 (see `proxy/src/wildcard.rs`). Per-series / per-identity
# subdomains all resolve here:
# - `series-<base32-key>.hubfederation.com` -> series content
# - `object-<hash>.hubfederation.com` -> raw object bytes
# - `collection-<hash>.hubfederation.com` -> snapshot item lookup
# - `edition-<id>.hubfederation.com` -> signed-edition item lookup
# - `<identity>.hubfederation.com` -> identity content
# - `hubfederation.com` -> proxy welcome
# The `testbed.hubfederation.com` record above is more-specific than
# the wildcard, so it stays the SSH-target name without colliding.
resource "digitalocean_record" "apex_ipv4" {
  domain = digitalocean_domain.hubfederation.id
  type   = "A"
  name   = "@"
  value  = digitalocean_droplet.samizdat_testbed.ipv4_address
}

resource "digitalocean_record" "apex_ipv6" {
  domain = digitalocean_domain.hubfederation.id
  type   = "AAAA"
  name   = "@"
  value  = digitalocean_droplet.samizdat_testbed.ipv6_address
}

resource "digitalocean_record" "wildcard_ipv4" {
  domain = digitalocean_domain.hubfederation.id
  type   = "A"
  name   = "*"
  value  = digitalocean_droplet.samizdat_testbed.ipv4_address
}

resource "digitalocean_record" "wildcard_ipv6" {
  domain = digitalocean_domain.hubfederation.id
  type   = "AAAA"
  name   = "*"
  value  = digitalocean_droplet.samizdat_testbed.ipv6_address
}
