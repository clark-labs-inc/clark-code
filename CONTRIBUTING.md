# Contributing to Agent Desktop

Thanks for helping improve Agent Desktop. Pull requests are welcome, but the
`main` branch is protected: changes require a passing CI run, the public-boundary
check, a maintainer review, resolved conversations, and a linear history.

Before opening a pull request:

- Run the relevant Rust and frontend checks from [AGENTS.md](AGENTS.md).
- Keep changes focused and preserve unrelated work in the shared checkout.
- Never commit credentials, customer data, authenticated screenshots, local
  paths, generated release artifacts, or changes copied from a private product
  repositories.
- Do not weaken sandbox, permission, credential-storage, or release checks to
  make a test pass. Explain contract changes in the pull request.
- Add or update tests for behavior changes.

Pull requests from forks run with read-only permissions. Maintainers may ask
for a smaller patch, additional evidence, or a clean-room provenance note
before merging.
