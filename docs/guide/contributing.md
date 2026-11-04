# Contributing

Bug reports, questions and reports of anything in these docs that's wrong are welcome in [GitHub issues](https://github.com/crenshawdev/terminus/issues). Tell me what you ran, what you expected and what happened. `terminus doctor` and `terminus status` output helps, once you've removed anything private. Terminus is an archive of your sessions, so check any excerpt before you paste it.

Pull requests are by invitation only, so open an issue first. Security problems go through [SECURITY.md](https://github.com/crenshawdev/terminus/blob/main/SECURITY.md), not a public issue. Issues opened from Windows are closed, and [TRUST.md](https://github.com/crenshawdev/terminus/blob/main/TRUST.md) says why.

## Tests

```sh
cargo test --workspace --all-features
```

CI runs them on every pull request and every push to `main`, on the release build on x86_64 Linux, aarch64 Linux and aarch64 macOS, and on the debug build on x86_64 Linux. They don't run on Intel Macs, where the release only builds the binary and checks that it installs. CI skips three tests of a hook spawn's p99 on every runner and three time budgets on a single spawn on the macOS runner, because a hosted VM's timing measures the VM. The tests over a real transcript corpus need `TERMINUS_TEST_CORPUS` and that private data, so they skip themselves in CI and on any machine without it.

`main` takes changes only through pull requests with signed commits, and only once the tests pass.

## Work on the book

The source is Markdown under `docs/guide/`. `SUMMARY.md` controls the navigation, and `docs/book.toml` configures the build. The files stay readable on GitHub. The design docs under `docs/design/` aren't part of the book, because they use Mermaid diagrams GitHub renders and mdBook doesn't.

With mdBook 0.5.4 installed, run from the repository root:

```sh
mdbook build docs
mdbook serve docs
```

The build writes `docs/book/`, which Git ignores. `serve` gives you a local preview. Check the rendered page as well as the Markdown, especially after changing tables or navigation.

Write from the reader's task. Say what a command does, what it changes and what the result establishes. Link to the design docs when the detail helps. A setting's description has to agree with the code that reads it, in [`config.rs`](https://github.com/crenshawdev/terminus/blob/main/crates/terminus-core/src/config.rs). Links out of the book use full GitHub URLs, because a relative path outside `docs/guide/` doesn't exist on the published site.

## Publishing the book

The documentation workflow builds pull requests that touch `docs/`. Its deployment job runs only on `main`, after a push there or a manual dispatch on that branch, and it uploads only the generated book.

Before the first deployment, select **GitHub Actions** as the repository's Pages publishing source and review the `github-pages` environment's deployment rules. The workflow doesn't enable Pages itself.

Contributions use the repository's MIT license.
