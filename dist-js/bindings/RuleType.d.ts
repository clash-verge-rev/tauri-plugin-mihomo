export type RuleType = "Domain" | "DomainSuffix" | "DomainKeyword" | "DomainRegex" | "DomainWildcard" | "GeoSite" | "GeoIP" | "SrcGeoIP" | "IPASN" | "SrcIPASN" | "IPCIDR" | "SrcIPCIDR" | "IPSuffix" | "SrcIPSuffix" | "SrcPort" | "DstPort" | "InPort" | "DSCP" | "InUser" | "InName" | "InType" | "ProcessName" | "ProcessPath" | "ProcessNameRegex" | "ProcessPathRegex" | "ProcessNameWildcard" | "ProcessPathWildcard" | "Match" | "RuleSet" | "Network" | "Uid" | "SubRules" | "AND" | "OR" | "NOT" | {
    "Unknown": string;
};
