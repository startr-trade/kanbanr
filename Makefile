# kanbanr — convenience targets.
# Layout: api/ (Rust workspace), web/ (React SPA), docker/, skill/, docs/, data/.
#
# Architecture: ONE binary. `kanbanr` is the local writer (the CLI, driven by the skill); it edits
# the data folder directly (git-backed). `kanbanr serve` runs the read-only view daemon over the
# same folder — localhost, no auth, no accounts. Sharing is via git remotes. Docker is optional.

# The board lives BESIDE the repo, not inside it (FEAT-041): a data folder inside a checkout is
# one `git add -A` away from being committed, and a sibling cannot be.
#
# ASK the CLI where it is rather than guessing the folder name. The `.kanbanr` marker is the
# authority, and a board may be named for its owner rather than for the repo — deriving
# `<repo>.kanbanr` was a guess that broke the moment one was renamed. The derivation stays only as
# a fallback for a fresh clone that has no CLI installed yet.
DATA_DIR ?= $(shell kanbanr where 2>/dev/null || echo $(CURDIR)/../$(notdir $(CURDIR)).kanbanr)
SKILLS_DIR ?= $(HOME)/.claude/skills
IMAGE ?= kanbanr:latest

.PHONY: help build test itest ci audit scan-deps scan-image codeql cli web check-docs install-cli install-skill install serve docker-build docker-up docker-down screenshots clean

help:
	@echo "Targets:"
	@echo "  build         Build the Rust workspace + web SPA"
	@echo "  test          Unit + Docker-less integration (CLI local writes + 'kanbanr serve' reads)"
	@echo "  itest         Packaging smoke: build the image + run the testcontainers test"
	@echo "  ci            Every CI check that can run off GitHub, before a push (scripts/ci-local.sh)"
	@echo "  audit         Supply chain: cargo deny + cargo audit + npm audit (fails on findings)"
	@echo "  scan-deps     Trivy over the tracked tree: deps, secrets, Dockerfile (fails on any finding)"
	@echo "  scan-image    Trivy over an image as a release scans it (IMAGE=, default kanbanr:ci-local)"
	@echo "  codeql        CodeQL for Rust, TypeScript and the workflows; several minutes"
	@echo "  install-cli   cargo install the one 'kanbanr' binary onto your PATH"
	@echo "  install-skill Symlink skill/kanbanr into ~/.claude/skills/"
	@echo "  install       install-cli + install-skill"
	@echo "  serve         Run the read-only view daemon (serves API + built SPA) on :8080"
	@echo "  docker-build  Build the single Docker image (runs 'kanbanr serve')"
	@echo "  docker-up     Build & run the view daemon container via docker compose"
	@echo "  docker-down   Stop the docker container"

build:
	cd api && cargo build --release
	cd web && npm install && npm run build

test:
	cd api && cargo test

# Packaging smoke: spin up the real Docker image via testcontainers and confirm it boots and
# serves the read-only view. Functional coverage is in `make test` (no Docker). The test is
# #[ignore]d, so build the image first.
itest: docker-build
	docker build -f docker/Dockerfile -t kanbanr:itest .
	cd api && cargo test -p kanbanr-cli -- --ignored

cli:
	cd api && cargo build --release -p kanbanr-cli

web:
	cd web && npm install && npm run build

# Every CI check that can run off GitHub, on a clean copy of the tracked tree, read from the
# workflow files themselves (FEAT-131). Run it before every push; it ends by listing what only
# GitHub can verify (the macOS and Windows legs, uploads, publishing).
ci:
	scripts/ci-local.sh

# The supply-chain gate CI's supply-chain job runs (FEAT-130): licences, sources, advisories.
audit:
	cd api && cargo deny check
	cd api && cargo audit
	cd web && npm audit --omit=dev

# The scanners trivy.yml, release.yml and codeql.yml run, locally (scripts/security-scan.sh). Each
# fails on a finding, where the workflows report: better seen here than on a public Security tab.
scan-deps:
	scripts/security-scan.sh deps

scan-image:
	scripts/security-scan.sh image $(IMAGE)

codeql:
	scripts/security-scan.sh codeql

# Every mermaid diagram in the docs parses with the library the monitor renders them with. A
# diagram that fails renders as nothing, which reads as a missing image rather than an error.
check-docs:
	cd web && npm install && npm run check:docs
	tools/docs/sync.sh
	git diff --exit-code -- docs/src/project/changelog.md \
		|| { echo "docs/src/project/changelog.md was stale: synced — commit it" >&2; exit 1; }

# Depends on `web`, because the monitor is compiled INTO the binary (FEAT-084): without the built
# SPA present, build.rs embeds nothing and `kanbanr serve` would quietly have no web view.
install-cli: web
	cd api && cargo install --path crates/kanbanr-cli
	@echo "Installed 'kanbanr'. Set your commit identity once:"
	@echo "  kanbanr identity --name \"You\" --email you@example.com"

install-skill:
	mkdir -p "$(SKILLS_DIR)"
	ln -sfn "$(CURDIR)/skill/kanbanr" "$(SKILLS_DIR)/kanbanr"
	@echo "linked $(SKILLS_DIR)/kanbanr -> $(CURDIR)/skill/kanbanr"

install: install-cli install-skill

# Local (no Docker) monitor: build the SPA then serve it read-only from the one binary.
# KANBANR_UI_DIR is passed deliberately even though the binary now embeds the monitor: during
# development you want the dist you just built, not whatever was baked in at compile time.
serve: web cli
	KANBANR_DATA_DIR="$(DATA_DIR)" \
	KANBANR_UI_DIR="$(CURDIR)/web/dist" \
	KANBANR_BIND="127.0.0.1:8080" \
	api/target/release/kanbanr serve

docker-build:
	docker build -f docker/Dockerfile -t $(IMAGE) .

# Compose cannot run a command to find the board, so pass it in from the same source everything
# else uses — the `.kanbanr` marker, read by the CLI.
docker-up:
	KANBANR_DATA_DIR="$(DATA_DIR)" docker compose -f docker/docker-compose.yml up --build -d
	@echo "kanbanr monitor at http://localhost:18080  (board: $(DATA_DIR))"

docker-down:
	KANBANR_DATA_DIR="$(DATA_DIR)" docker compose -f docker/docker-compose.yml down

# Regenerate docs/images/*.png by driving the live monitor through a Selenium Grid (Docker).
# Needs Docker; starts a temporary `kanbanr serve` if one isn't already running. See
# tools/screenshots/README.md.
screenshots:
	tools/screenshots/capture.sh

clean:
	cd api && cargo clean
	cd tools/screenshots && cargo clean
	rm -rf web/dist web/node_modules editor/vscode/node_modules
