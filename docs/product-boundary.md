# Product boundary

This repository is the open desktop foundation. It owns provider-neutral domain
types, projection, local execution, sandboxing, the Tauri lifecycle, and generic
UI primitives. A branded distribution is a separate composition project.

## Dependency direction

The dependency arrow is one way:

```text
private product -> open desktop foundation
```

The foundation must never import a product crate, product provider, private
service schema, plan name, billing rule, hosted origin, signing identity, or
specialist catalog. A product may implement the public extension contracts and
pin a reviewed foundation revision.

## Native extension contracts

- `ProductIntegration::make_provider` constructs product-owned provider
  adapters. The neutral host constructs only its built-in local and ACP
  providers.
- `ProductIntegration::prepare_provider_config` binds native credentials and
  account scope. Credentials never cross the WebView boundary.
- `ProductIntegration::request` exposes a bounded opaque operation namespace
  for product services. The renderer receives projections, not private service
  schemas.
- `ProductIntegration::publish_projection` handles product trace projections
  without teaching the foundation about specialist routes or schemas.
- `ToolPack` installs product-owned tools into the local agent without allowing
  them to shadow foundation tools.

## Renderer extension contracts

`ProductModule` supplies branding, optional UI slots, usage-failure rendering,
specialist access copy, badges and icons, model/catalog policy, and gated
workflow definitions.
Product availability is represented by `ProductAccessProjection`: an opaque
capability id plus server-authored availability, explanation, and action. The
renderer must not reconstruct subscription, credit, seat, or entitlement
policy. The neutral composition exposes no commercial workflow catalog.

The neutral build uses an empty specialist catalog. A branded entry may install
a signed catalog before the application modules load.

## What belongs outside this repository

- hosted product transport and endpoint allowlists;
- authentication, cloud history, billing, and account schemas;
- plan, credit, seat, and subscriber-workflow policy;
- hosted research, Scout enrollment/routing policy, Security cloud transport,
  and specialist product catalogs;
- product branding, deep links, updater keys/endpoints, bundle identifiers, and
  release signing configuration;
- proprietary product documentation and simulations.

## Enforcement

Every product hook is optional and the neutral implementation fails closed.
Extension tool registration rejects invalid names and collisions. Product
configuration is accepted only after the host rejects renderer-supplied
credentials. Product composition builds independently so its lockfile and
release dependencies cannot contaminate the foundation workspace.
`harness/product-boundary.spec.mjs` rejects product policy and stale product
aliases in the foundation; downstream compositions should maintain a matching
guard against duplicate authorities and cross-repository source inclusion.

## Stable protocol identifiers

The open Scout crates include portable protocol types, deterministic adapters,
and cryptographic verification. A small number of `clark.*` signing domains and
`clark/...@1` adapter identifiers are immutable versioned wire identifiers used
by already-issued receipts. They are retained strictly for interoperability;
service routes, tenant resolution, credentials, enrollment policy, and hosted
authorization remain downstream product concerns. Changing those identifiers
would define a new protocol version rather than improve the product boundary.
