import type { JsonValue } from "./serde_json/JsonValue";
export type Log = {
    type: string;
    payload: string;
} & ({
    [key in string]: number | string | boolean | Array<JsonValue> | {
        [key in string]: JsonValue;
    } | null;
});
