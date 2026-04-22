import type { AppliedGroup } from "./AppliedGroup";
/**
 * PUT /network/context response body.
 *
 * `expiresAt` is always present on the wire: `null` means the context is
 * sticky (no TTL); a concrete number is the absolute expiry timestamp in
 * unix seconds.
 */
export type PutResponse = {
    matchedNetwork: string | null;
    applied: Array<AppliedGroup>;
    expiresAt: number | null;
};
