import type { ProviderType } from "./ProviderType";
import type { RuleBehavior } from "./RuleBehavior";
import type { RuleFormat } from "./RuleFormat";
import type { VehicleType } from "./VehicleType";
import type { JsonValue } from "./serde_json/JsonValue";
export type RuleProvider = {
    behavior: RuleBehavior;
    format: RuleFormat;
    name: string;
    ruleCount: number;
    type: ProviderType;
    updatedAt: string;
    vehicleType: VehicleType;
} & ({
    [key in string]: number | string | boolean | Array<JsonValue> | {
        [key in string]: JsonValue;
    } | null;
});
