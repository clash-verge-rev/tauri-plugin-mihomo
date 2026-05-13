/**
 * Per-group state snapshot in the GET /network/context response.
 *
 * `lastMatchedNetwork` is either a concrete network name or JSON null.
 * The internal sentinel `"<none>"` never appears on the wire; it is always
 * encoded as null. When null, `selectionSource` only partially
 * disambiguates the cause:
 * - `selectionSource = "unknown"` + null → never evaluated
 * - `selectionSource = "auto"` + null → evaluated, matched no network
 * - `selectionSource = "manual"` + null → ambiguous: either the group was
 *   manually set before any evaluation, or evaluation found no match and
 *   was then overridden manually. Callers that must distinguish these
 *   have to track host-side interaction history.
 */
export type GroupStatus = {
    group: string;
    currentProxy: string;
    selectionSource: string;
    lastMatchedNetwork: string | null;
};
