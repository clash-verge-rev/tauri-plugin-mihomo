import type { RuleType } from "./RuleType";
import type { JsonValue } from "./serde_json/JsonValue";
export type Rule = {
    type: RuleType;
    index: number;
    payload: string;
    proxy: string;
    size: number;
} & ({
    [key in string]: number | string | boolean | Array<JsonValue> | {
        [key in string]: JsonValue;
    } | null;
});
