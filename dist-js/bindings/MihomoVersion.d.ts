import type { JsonValue } from "./serde_json/JsonValue";
/**
 * mihomo version
 */
export type MihomoVersion = {
    meta: boolean;
    version: string;
} & ({
    [key in string]: number | string | boolean | Array<JsonValue> | {
        [key in string]: JsonValue;
    } | null;
});
