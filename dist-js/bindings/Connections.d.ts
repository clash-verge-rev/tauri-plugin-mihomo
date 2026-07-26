import type { Connection } from "./Connection";
import type { JsonValue } from "./serde_json/JsonValue";
/**
 * connections
 */
export type Connections = {
    downloadTotal: number;
    uploadTotal: number;
    connections: Array<Connection> | null;
    memory: number;
} & ({
    [key in string]: number | string | boolean | Array<JsonValue> | {
        [key in string]: JsonValue;
    } | null;
});
