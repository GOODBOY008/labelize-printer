# Labelize dependency snapshot

Upstream: https://github.com/GOODBOY008/labelize

Version: 1.10.0, with complete-job parser API from commit
`1ee9ef511874d38d989b7a938e3cfc9ad6576ac2` on local branch
`codex/virtual-printer-parser-options` (baseline `56dc51c5f83b1996312c9916078c799914713845`).

The snapshot contains `src/`, Cargo.toml, README and upstream license notices.
It is used as a library with default features disabled. The parser API is the
only upstream change required by the independent printer project. Replace this
path dependency with a released Labelize version when that API is published.

Do not make platform or job-management changes in this snapshot. Parser fixes
belong upstream, with the Labelize golden regression suite, followed by an exact
snapshot refresh. The standalone crate does not include upstream test data.

Upstream checks for this API: 50 existing ZPL parser tests, 5 job-option tests,
3 architecture tests, 170 golden tests, and 3 diff-report tests passed. Regenerated
carrier/unit/reverse artifacts are unchanged.
