import type { JsonValue } from "./serde_json/JsonValue";
export type Traffic = {
    up: number;
    down: number;
    upTotal: number;
    downTotal: number;
} & ({
    [key in string]: number | string | boolean | Array<JsonValue> | {
        [key in string]: JsonValue;
    } | null;
});
