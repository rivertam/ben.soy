# Thought scaffolding and publishing.
#   just thought new       scaffold a post with register_post! (interactive or with args)
#   just thought publish   commit + push pending thought changes

default:
    @just --list thought

# Scaffold a new thoughts post (page module + mod + register_post!; date = today).
# No args → interactive prompts; or pass slug / title / teaser / tags.
# Tags are comma-separated lowercase labels (required; indexes assert non-empty).
new slug="" title="" teaser="" tags="":
    bash {{justfile_directory()}}/scripts/thought-new.sh {{quote(slug)}} {{quote(title)}} {{quote(teaser)}} {{quote(tags)}}

# Commit and push pending changes under thoughts (register_post! lives in the page).
publish:
    bash {{justfile_directory()}}/scripts/thought-publish.sh
