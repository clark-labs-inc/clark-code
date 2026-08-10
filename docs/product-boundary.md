# Extension architecture

Clark Code keeps its agent domain, execution policy, and interface independent
from any single model provider or hosted service. Optional integrations connect
through explicit native and renderer contracts.

## Native extensions

- `ProductIntegration::make_provider` constructs an additional provider.
- `ProductIntegration::prepare_provider_config` binds native-only credentials
  and account scope without exposing secrets to the WebView.
- `ProductIntegration::request` exposes bounded integration operations while
  keeping service schemas out of the shared renderer contract.
- `ProductIntegration::publish_projection` accepts typed trace projections.
- `ToolPack` installs additional local-agent tools without allowing them to
  shadow built-in tools.

## Renderer extensions

`ProductModule` can provide branding, interface slots, model choices,
capability projections, specialist catalogs, icons, and workflow definitions.
The default Clark Code module remains usable without any optional integration.

Availability is represented by `ProductAccessProjection`: a capability id,
availability state, explanation, and optional action. The renderer consumes
that projection instead of reconstructing remote access policy.

## Safety rules

- Credentials remain in the native host.
- Extension tool names cannot collide with built-in tools.
- Provider configuration is validated before a session starts.
- Hosted routes and deployment credentials are not compiled into the default
  application.
- The shared event projection remains pure and provider-independent.

The repository guard in `harness/product-boundary.spec.mjs` checks these
constraints across source, tests, documentation, fixtures, and configuration.

## Stable wire identifiers

Some Scout protocol crates retain versioned signing domains and adapter ids
used by existing receipts. They are interoperability identifiers, not service
configuration. Changing them requires a new protocol version.
