---
name: sentry
description: Inspect current Sentry issues and events, summarize recent production errors, or gather read-only Sentry health evidence. Use for `/sentry` requests, Sentry issue IDs, production-error triage, release regressions, and recent event investigation.
---

# Inspect Sentry read-only

Query Sentry at the time of the request. Treat repository metadata, remembered
issues, and prior output as leads rather than current production evidence.

## Keep the boundary read-only

- Use only Sentry GET endpoints. Never resolve, ignore, assign, delete, or mutate
  issues, alerts, projects, releases, or organization settings.
- Match the user's scope. Inspection, diagnosis, triage, and summaries do not
  authorize source edits or deployments.
- Do not print auth tokens, put them in URLs, or paste them literally into tool
  arguments. Reference `SENTRY_AUTH_TOKEN` through the environment.
- Redact email addresses, IP addresses, user identifiers, cookies, request
  bodies, and other personal or secret values before showing output.
- Do not dump raw stack traces. Extract only the frames needed to identify the
  first application-owned failure and cite the event or issue URL when present.

## Authenticate without exposing credentials

Use `bash` to check whether `SENTRY_AUTH_TOKEN` is set without echoing its value.
Prefer `SENTRY_ORG`, `SENTRY_PROJECT`, and `SENTRY_BASE_URL`; default the base URL
to `https://sentry.io`. Org and project may also be inferred from checked-in
Sentry configuration, but never print credentials found there.

If the token is missing, stop before making a request. Tell the user to create a
read-only token with `org:read`, `project:read`, and `event:read`, set it locally
as `SENTRY_AUTH_TOKEN`, and retry. Never ask them to paste the token into chat.

## Query the narrow endpoint

Use a connected read-only Sentry tool when one is available. Otherwise call the
REST API from `bash` with Python's standard library or `curl`. URL-encode path and
query values, send `Authorization: Bearer $SENTRY_AUTH_TOKEN`, request JSON, and
keep the maximum result count at 50. Honor Sentry's `Link` cursor when more than
one page is required.

- List issues:
  `GET /api/0/projects/{org}/{project}/issues/`
  with `statsPeriod`, `environment`, `query`, and `per_page`.
- Resolve a short ID such as `APP-123` by listing issues with that ID as the
  search query, then use the returned numeric issue ID.
- Issue detail:
  `GET /api/0/organizations/{org}/issues/{issue_id}/`.
- Events for an issue:
  `GET /api/0/organizations/{org}/issues/{issue_id}/events/`
  with `statsPeriod`, `environment`, and `per_page`.
- Event detail:
  `GET /api/0/projects/{org}/{project}/events/{event_id}/`.

When `/sentry` has no narrower request, list the 20 most recently seen
unresolved issues in `prod` from the last `24h`.

## Report evidence, not a data dump

For issue lists, report title, short ID, status, count, first seen, last seen,
environments, and the most useful tags, ordered by most recent. For one event,
report timestamp, environment, release, culprit, URL, and the smallest useful
application-frame evidence. State explicitly when no results match.

Separate what Sentry directly proves from any inference about the current source
or deployed release. End with the queried org/project, environment, time range,
and any access or data gap that limits the conclusion.
