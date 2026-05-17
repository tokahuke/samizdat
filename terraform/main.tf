terraform {
  required_providers {
    digitalocean = {
      source  = "digitalocean/digitalocean"
      version = "~> 2.0"
    }
    github = {
      source  = "integrations/github"
      version = "~> 6.0"
    }
    tls = {
      source  = "hashicorp/tls"
      version = "~> 4.0"
    }
  }

  cloud {
    organization = "samizdat"

    workspaces {
      name = "Samizdat"
    }
  }
}

variable "do_token" {
  type        = string
  description = "DigitalOcean API token."
}

variable "github_token" {
  type        = string
  description = "GitHub PAT with `repo` scope so terraform can push action secrets."
  sensitive   = true
}

variable "github_owner" {
  type        = string
  description = "GitHub user or org that owns the samizdat repo."
}

variable "github_repo" {
  type        = string
  description = "Name of the samizdat repo on GitHub (without owner)."
  default     = "samizdat"
}

variable "get_samizdat_repo" {
  type        = string
  description = "Name of the get-samizdat release-collection repo (where postbuild.sh pushes built artifacts)."
  default     = "get-samizdat"
}

variable "proxy_owner_email" {
  type        = string
  description = "Contact email registered with Let's Encrypt for the proxy's TLS cert."
}

provider "digitalocean" {
  token = var.do_token
}

provider "github" {
  token = var.github_token
  owner = var.github_owner
}
