# Security policy for this fixture

Review first-party files under `src/`. Treat `vendor/` as generated third-party
code and exclude it with a reason. Test files are supporting evidence.

Assume an unauthenticated internet attacker unless a route explicitly requires
a session. A normal tenant user must not read or mutate another tenant's data,
select server-side network destinations, influence shell commands, or escape
the configured file and archive roots.
