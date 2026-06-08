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

resource "digitalocean_record" "proxy_ipv4" {
  domain = digitalocean_domain.hubfederation.id
  type   = "A"
  name   = "proxy"
  value  = digitalocean_droplet.samizdat_testbed.ipv4_address
}

resource "digitalocean_record" "proxy_ipv6" {
  domain = digitalocean_domain.hubfederation.id
  type   = "AAAA"
  name   = "proxy"
  value  = digitalocean_droplet.samizdat_testbed.ipv6_address
}

# Per-series subdomain isolation at the proxy. Each series is served at
# `<base32-key>.proxy.hubfederation.com` and each identity at
# `<handle>.proxy.hubfederation.com`. The wildcard A/AAAA points every
# such name at the same droplet; the proxy obtains TLS certs on demand
# per SNI via ACME HTTP-01.
resource "digitalocean_record" "proxy_wildcard_ipv4" {
  domain = digitalocean_domain.hubfederation.id
  type   = "A"
  name   = "*.proxy"
  value  = digitalocean_droplet.samizdat_testbed.ipv4_address
}

resource "digitalocean_record" "proxy_wildcard_ipv6" {
  domain = digitalocean_domain.hubfederation.id
  type   = "AAAA"
  name   = "*.proxy"
  value  = digitalocean_droplet.samizdat_testbed.ipv6_address
}
