import type { GroupStatus } from "./GroupStatus";
import type { NetworkContext } from "./NetworkContext";
/**
 * GET /network/context response body.
 *
 * `context = null` is the authoritative "no ctx" signal (cold start,
 * DELETE, or TTL expiry); `groups` is always returned so callers can render
 * per-group state regardless of ctx presence.
 *
 * `matchedNetwork`, `expiresAt`, `ageSeconds` are always present on the
 * wire, using explicit JSON null when absent. `ageSeconds = null` is a
 * reliable ctx-absent marker. `expiresAt = null` is ambiguous — it can
 * mean either "sticky ctx" or "no ctx" — and must be disambiguated through
 * `context`.
 */
export type NetworkStatus = {
    context: NetworkContext | null;
    matchedNetwork: string | null;
    groups: Array<GroupStatus>;
    expiresAt: number | null;
    ageSeconds: number | null;
};
