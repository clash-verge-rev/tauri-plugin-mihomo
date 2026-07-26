import type { MuxOption } from "./MuxOption";
import type { JsonValue } from "./serde_json/JsonValue";
export type TuicServer = {
    enable: boolean;
    listen: string;
    token?: Array<string>;
    users?: {
        [key in string]: string;
    };
    clientAuthType?: string;
    clientAuthCert?: string;
    certificate: string;
    privateKey: string;
    echKey: string;
    congestionController?: string;
    maxIdleTime?: number;
    authenticationTimeout?: number;
    alpn?: Array<string>;
    maxUdpRelayPacketSize?: number;
    maxDatagramFrameSize?: number;
    cwnd?: number;
    bbrProfile?: string;
    muxOption?: MuxOption;
} & ({
    [key in string]: number | string | boolean | Array<JsonValue> | {
        [key in string]: JsonValue;
    } | null;
});
