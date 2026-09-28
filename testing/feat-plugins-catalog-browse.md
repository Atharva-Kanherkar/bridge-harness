# feat/plugins-catalog-browse — Test Contract

Issue: Atharva-Kanherkar/bridge-harness#691
"Plugins screen dumps the entire catalog and shows unnamed app-<id> rows"

Locked before implementation. A later step may not silently redefine "done";
if the contract changes, it changes in its own commit first.

## Functional Behavior

The Plugins screen (`MarketplaceScreen` → `PluginMarketplace`) must stop being a
single dump of every provider catalog entry.

1. **No unnamed row ever renders as a title.**
   - A variant whose display name fell back to the provider id
     (`name_is_fallback == true`) is *hidden* until a real name is known.
   - Codex app connectors get a real name, description, icon and category by
     joining the `app/list` response the CLI already exposes
     (plugin id `app-<hex>@…` ↔ app id `asdk_app_<hex>`; bundled plugins join by
     `appConnectorIds` ↔ connector id).
   - A variant that is renamed is no longer a fallback.
   - A service is hidden iff every variant in it is a fallback; a service whose
     fallback variant is later named shows the human name.
   - A resolved identity also carries the app's authentication verdict, and a
     Codex plugin id shaped `app-<hex>` is classified as an app connector even
     though the catalog listing has no connector metadata for it. The detail
     page therefore shows `Needs login` and a working `Connect` action for an
     installed app whose authorization needs renewing.

2. **Default view is small: installed + featured.**
   - View tabs: `Installed` · `Featured` · `All`.
   - `Featured` = the installed services plus a "Popular" set (at most
     `FEATURED_LIMIT = 12` services ranked by maximum `installCount`, excluding
     already-installed ones), plus an explicit entry point to `All`.
   - Opening Plugins with no query mounts at most a few dozen rows. It never
     mounts the long tail.

3. **Categories divide the catalog.**
   - A service's category comes from its variants' provider-reported category
     (`Codex app/list appMetadata.categories[0]`, delivered through app auth
     states and applied by `applyAppAuthStates`).
   - `All` shows category filter chips (`All`, each category by service count,
     `Other` last when uncategorized services exist). Selecting one filters the
     list; selecting `All` shows everything.
   - `categoryLabel` renders `DEVELOPER_TOOLS` as `Developer tools`.

4. **The long tail is reached by search or an explicit browse, progressively.**
   - Search matches name, description and capabilities across the *whole* named
     catalog, not just the featured set.
   - `All` and search results render one page at a time (`PAGE_SIZE = 24`) with
     a "Show more" control; only the current page is mounted.
   - Changing view/category/provider/scope/query resets pagination.

5. **Loading is honest.**
   - While the first catalog is in flight, the screen shows a list skeleton (not
     a bare spinner) plus the existing provider error banners.
   - While app identities are still resolving (catalog present, app states not
     yet), the screen says `Resolving connector names…`.
   - A failed identity load keeps identities unresolved instead of pretending
     there were none: the screen says so, offers `Retry`, retries on the
     Refresh control, and the background poll retries until identities load.
   - A refresh keeps the current list mounted and only spins the refresh control.

Out of scope (deliberate):
- Caching the catalog in core with a TTL. The CLI catalog call is fast
  relative to the app/list call; revisit separately.
- Changing `forceRefetch`/polling behaviour of `app_auth_states`.
- Wire/protocol changes: all touched result types are `DEFERRED_RESULTS`
  (`MarketplaceCatalog`, `MarketplaceAppAuthState`), so adding optional fields
  needs no protocol-mirror change. `MarketplaceCatalog` stays parameterless.
- Pagination/virtualization inside a service detail page.

## Unit Tests

`src-tauri/bridge-core/src/marketplace.rs` (cargo test):
- `names_that_fall_back_to_the_provider_id_are_marked` — a JSON entry with only
  `id` yields `name == id` and `name_is_fallback == true`; one with
  `displayName` yields `name_is_fallback == false`.
- `parses_only_explicit_app_accessibility_states` (updated) — Codex app states
  carry `display_name`, `description`, `icon_url` (https only) and `category`;
  entries without `name`/`installUrl` are still dropped.
- `app_identity_icons_must_be_absolute_https_urls` — relative icon paths are
  rejected.
- `remote_app_plugin_ids_are_classified_as_app_connectors` — a Codex
  `app-<hex>@…` entry parses with `connector_type == "app"` and no portable MCP
  fallback, so authentication can route through the app installer.
- `merge_variant` promotes a real name over a fallback name and keeps the
  fallback flag only while both sides are fallbacks.

`src/marketplace.test.ts` (vitest):
- `applyAppAuthStates` renames a Codex `app-<hex>` fallback from an
  `asdk_app_<hex>` state and clears `nameIsFallback`.
- `applyAppAuthStates` enriches description/icon/category only when the variant
  lacks them, and never renames a variant that already has a human name.
- `applyAppAuthStates` carries a hex-matched state's verdict onto the resolved
  row (`required` and `connected`), with `required` still winning when a direct
  connector state disagrees.
- `isUnnamedVariant` / `isUnnamedService` semantics: fallback flag or empty
  name; service hidden only when every variant is unnamed.
- `categoryLabel`: `DEVELOPER_TOOLS` → `Developer tools`; blank stays blank.
- `servicePopularity` reads `installCount` from `providerMetadata`.
- `groupMarketplaceServices` upgrades a service's name from a fallback to a
  human name when a later variant has one.

## Integration / Functional Tests

`src/components/MarketplaceScreen.test.tsx` (vitest + jsdom), rendering the real
`MarketplaceScreen` and switching to the `plugins` catalog tab:
- default view is Installed + Featured: installed fixture present, top
  `installCount` fixtures present, a zero-install long-tail fixture absent, and
  mounted `article` rows stay under the cap.
- unnamed variants hidden / resolved: an `app-<hex>` fallback fixture never
  renders its id; once the matching app auth state carries a display name the
  row appears under the human name.
- recovery actions: an installed, resolved app connector with a `required`
  state shows `Needs login` and a `Connect` action on its detail page.
- identity retry: when the first app-state call rejects, the screen says names
  could not be resolved, the installed app stays hidden, and refreshing retries
  the identity load and reveals it under its real name.
- search reaches the long tail: querying a non-featured service's name and a
  capability-ish description shows the row.
- All view paginates: only `PAGE_SIZE` rows mount, "Show more" appends the next
  page.
- categories: chips derive from variant categories; selecting a chip filters
  rows.
- loading: with `marketplaceCatalog` unresolved, the skeleton
  (`[data-testid="plugin-catalog-skeleton"]`) renders instead of the list.

## Smoke Tests

- `bun run build` (tsc -b + vite build) is green.
- `bun run test` (vitest + cargo test workspace) is green.
- `cargo test -p bridge-core marketplace` covers the Rust unit tests above.

## E2E Tests

N/A — not applicable for this change. The Tauri shell has no browser E2E suite;
the component tests above render the real screen tree.

## Manual / cURL Tests

1. `bun run tauri dev`, open Plugins → the screen paints a skeleton, then
   installed + Popular rows; the full catalog is not mounted.
2. Search `gmail` → after `Resolving connector names…` clears, Gmail resolves
   from `app/list` and is reachable; no `app-<hex>` title ever appears.
3. Switch to All → at most 24 rows mount; "Show more" appends.
4. Pick a category chip (e.g. `Productivity`) → only that category's named
   services show.
5. Native checks (read-only):
   - `claude plugin list --available --json` — 300+ named entries, each with
     `installCount`.
   - `codex plugin list --available --json` — thousands of entries, many
     `app-<hex>@openai-curated-remote`.
   - `codex app-server` → `app/list` — every entry carries `name` and
     `installUrl`; some carry `appMetadata.categories` and absolute icon URLs.

## Acceptance Mapping

| Issue acceptance | Covered by |
| --- | --- |
| No bare `app-<hex>` title | Rust fallback flag + TS hide/rename + screen tests |
| ≤ a few dozen rows on open | Featured limit + pagination + screen test |
| Search finds any entry | search over all named services + screen test |
| Vitest coverage in MarketplaceScreen | `src/components/MarketplaceScreen.test.tsx` |
| Build + tests green | smoke stage |
