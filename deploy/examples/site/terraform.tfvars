# Ordinary settings, committed. Secrets never go here: put them in
# terraform.sensitive.auto.tfvars (see the .example beside this file).
# The .invalid names can never resolve; replace them with your own.
namespace    = "kanbanr-site"
image        = "ghcr.io/startr-trade/kanbanr:v0.8.0"
ingress_host = "boards.example.invalid"
boards = [
  { name = "board-a", url = "https://git.example.invalid/team/board-a.git" },
  { name = "board-b", url = "https://git.example.invalid/team/board-b.git", pull_interval_seconds = 120 },
]
