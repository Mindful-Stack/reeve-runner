.PHONY: help setup pull status split-lore rename build-index validate doctor test

help:    ## List targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-20s\033[0m %s\n", $$1, $$2}'

setup:   ## Clone every sibling declared in household.json
	@./scripts/setup.sh

pull:    ## Fetch all siblings; ff-pull if on clean main
	@./scripts/pull-all.sh

status:  ## One-line git status per sibling
	@./scripts/status-all.sh

split-lore: ## Promote the inline `lore/` to its own sibling repo (interactive; pass REMOTE=<url> to skip prompt)
	@./scripts/split-lore.sh "$(REMOTE)"

rename:  ## Substitute placeholder workspace name (usage: make rename NAME=foo)
	@./scripts/rename.sh "$(NAME)"

build-index: ## Rebuild lore/knowledge/_index.json
	@node lore/_tools/cli.js build-index --dir lore/knowledge

validate: ## Run KB validators (frontmatter, links, orphans)
	@node lore/_tools/cli.js validate --dir lore/knowledge

doctor:  ## Run full workspace + KB diagnostic
	@node lore/_tools/cli.js doctor --dir lore/knowledge

test:    ## Run lore tooling unit tests
	@node --test lore/_tools/__tests__/*.test.js
