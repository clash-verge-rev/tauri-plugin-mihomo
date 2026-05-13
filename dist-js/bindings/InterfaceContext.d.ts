/**
 * Per-interface entry in `NetworkContext.interfaces`. `name` is the only
 * required field; all per-iface attributes are optional.
 */
export type InterfaceContext = {
    name: string;
    iface_type?: string;
    ssid?: string;
    bssid?: string;
    gateway_ip?: string;
    gateway_mac?: string;
    subnets?: Array<string>;
    metered?: boolean;
};
