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
