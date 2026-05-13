/**
 * Per-group decision carried in the PUT /network/context response.
 *
 * `selectionSource` and `reason` are free-form strings on the wire. Current
 * mihomo values:
 * - `selectionSource`: `auto` / `manual` / `unknown`
 * - `reason`: `matched` / `already_selected` / `default` /
 *   `no_change_no_default` / `unchanged_network` / `manual_locked` /
 *   `missing_target`
 */
export type AppliedGroup = {
    group: string;
    targetProxy: string | null;
    appliedProxy: string;
    changed: boolean;
    selectionSource: string;
    reason: string;
};
