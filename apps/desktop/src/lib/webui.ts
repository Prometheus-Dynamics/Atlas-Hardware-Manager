// Where a device's web page can be opened. The board reports its page by
// hostname (`http://raze-ab12cd34.local:5800/`), which needs the computer
// to resolve mDNS names; many laptops don't. The same page by IP always
// works from here, since that is how Atlas reaches the device.
import type { DeviceRecord } from "#lib/api/types.ts";

export interface WebUiTarget {
  url: string;
  /** What the menu shows: the host, `raze-ab12cd34.local` or `10.0.0.5`. */
  host: string;
  kind: "ip" | "name";
}

function hostOf(address: string): string | null {
  try {
    return new URL(address).hostname || null;
  } catch {
    return /^[\d.]+$/.test(address) ? address : null;
  }
}

const isIpv4 = (host: string) => /^\d{1,3}(\.\d{1,3}){3}$/.test(host);

/** Every way to open the device's web page, the first one to use first:
 *  by the IP Atlas reaches it on, the board's other IPs, then by name. */
export function webUiTargets(record: DeviceRecord): WebUiTarget[] {
  const attrs = record.identity.attributes ?? {};
  const reported = attrs.manage_url;
  if (!reported) return [];
  let base: URL;
  try {
    base = new URL(reported);
  } catch {
    return [{ url: reported, host: reported, kind: "name" }];
  }
  const ips = [hostOf(record.identity.address), ...(attrs.addresses ?? "").split(",")]
    .map((ip) => ip?.trim() ?? "")
    .filter((ip, i, all) => isIpv4(ip) && all.indexOf(ip) === i && ip !== base.hostname);
  const byIp = ips.map((ip) => {
    const url = new URL(base);
    url.hostname = ip;
    return { url: url.toString(), host: ip, kind: "ip" as const };
  });
  const asReported = { url: base.toString(), host: base.hostname, kind: isIpv4(base.hostname) ? ("ip" as const) : ("name" as const) };
  return [...byIp, asReported];
}
