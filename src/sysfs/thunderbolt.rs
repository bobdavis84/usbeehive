//! Thunderbolt / USB4 router enumeration from `/sys/bus/thunderbolt/devices/`.

use std::path::{Path, PathBuf};

use crate::thunderbolt::{parse_speed_gbps, ThunderboltDomain, ThunderboltRouter};

use super::reader::{self, Sysfs};

impl Sysfs {
    /// Walk this sysfs root's Thunderbolt devices directory and return every
    /// router (host adapters and connected devices). Empty `Vec` when the
    /// subsystem isn't loaded.
    pub fn thunderbolt_routers(&self) -> Vec<ThunderboltRouter> {
        enumerate_in(&self.thunderbolt_dir())
    }

    /// Walk the Thunderbolt devices directory and return every `domainN`
    /// entry with its security level. Empty `Vec` when the subsystem isn't
    /// loaded.
    pub fn thunderbolt_domains(&self) -> Vec<ThunderboltDomain> {
        enumerate_domains_in(&self.thunderbolt_dir())
    }

    /// `true` when the Thunderbolt bus directory exists at all — lets
    /// callers tell "subsystem not loaded" from "nothing attached".
    pub fn has_thunderbolt_bus(&self) -> bool {
        reader::path_exists(self.thunderbolt_dir())
    }
}

/// Parse a boolean-ish attribute (`0` / `1`; any non-zero integer is true).
/// Anything else is reported as absent.
fn read_flag(path: PathBuf) -> Option<bool> {
    reader::read_int(path).map(|v| v != 0)
}

fn from_sysfs(path: &Path, name: &str) -> Option<ThunderboltRouter> {
    // Linux `switch.c` names router devices `<decimal domain>-<hex route>`;
    // route bytes are packed least-significant hop first. ABI retimer devices
    // use `<device>:<port>.<index>` and are not routers, so reject their colon.
    let (domain_str, route_str) = name.split_once('-')?;
    let domain: u32 = domain_str.parse().ok()?;
    if route_str.is_empty()
        || route_str.len() > 16
        || !route_str.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    let route = u64::from_str_radix(route_str, 16).ok()?;
    let generation_attr = reader::read_int(path.join("generation"));
    let has_router_identity = reader::read_hex(path.join("vendor")).is_some()
        && reader::read_hex(path.join("device")).is_some();
    // Thunderbolt service devices can share the bus directory without being
    // routers. When generation is absent, require the router's vendor/device
    // ABI attributes before treating its route-like name as a router.
    if generation_attr.is_none() && !has_router_identity {
        return None;
    }
    let reported_generation = generation_attr
        .and_then(|v| u8::try_from(v).ok())
        .filter(|v| *v > 0);
    let generation = reported_generation.unwrap_or(0);

    // Host adapter route always terminates in `-0` (`0-0`, `1-0`, …).
    let is_host = route == 0;

    let vendor_id = reader::read_hex(path.join("vendor"))
        .map(|v| (v & 0xFFFF) as u16)
        .unwrap_or(0);
    let device_id = reader::read_hex(path.join("device"))
        .map(|v| (v & 0xFFFF) as u16)
        .unwrap_or(0);

    Some(ThunderboltRouter {
        sysfs_path: path.to_path_buf(),
        route: name.to_string(),
        domain,
        is_host,
        generation,
        reported_generation,
        vendor_id,
        device_id,
        vendor_name: reader::read_attr(path.join("vendor_name")).unwrap_or_default(),
        device_name: reader::read_attr(path.join("device_name")).unwrap_or_default(),
        unique_id: reader::read_attr(path.join("unique_id")).unwrap_or_default(),
        raw_attributes: reader::read_all_attrs(path),
        authorized: read_flag(path.join("authorized")),
        nvm_version: reader::read_attr(path.join("nvm_version")),
        rx_speed_gbps: reader::read_attr(path.join("rx_speed"))
            .as_deref()
            .and_then(parse_speed_gbps),
        rx_lanes: reader::read_int(path.join("rx_lanes")).and_then(|v| u8::try_from(v).ok()),
        tx_speed_gbps: reader::read_attr(path.join("tx_speed"))
            .as_deref()
            .and_then(parse_speed_gbps),
        tx_lanes: reader::read_int(path.join("tx_lanes")).and_then(|v| u8::try_from(v).ok()),
    })
}

pub(crate) fn enumerate_domains_in(base: &Path) -> Vec<ThunderboltDomain> {
    if !reader::path_exists(base) {
        return Vec::new();
    }
    let mut out: Vec<ThunderboltDomain> = reader::subdirs(base)
        .into_iter()
        .filter_map(|p| {
            let name = p.file_name()?.to_string_lossy().into_owned();
            let domain: u32 = name.strip_prefix("domain")?.parse().ok()?;
            Some(ThunderboltDomain {
                name,
                domain,
                security: reader::read_attr(p.join("security")),
                iommu_dma_protection: read_flag(p.join("iommu_dma_protection")),
                deauthorization: read_flag(p.join("deauthorization")),
            })
        })
        .collect();
    out.sort_by_key(|d| d.domain);
    out
}

pub(crate) fn enumerate_in(base: &Path) -> Vec<ThunderboltRouter> {
    if !reader::path_exists(base) {
        return Vec::new();
    }
    let mut entries: Vec<(PathBuf, String)> = reader::subdirs(base)
        .into_iter()
        .filter_map(|p| {
            let name = p.file_name()?.to_string_lossy().into_owned();
            // Skip subsystem links (`domain0`, etc.) and anything that
            // doesn't look like a route string.
            if !name.contains('-') {
                return None;
            }
            Some((p, name))
        })
        .collect();
    entries.sort_by(|a, b| a.1.cmp(&b.1));
    entries
        .iter()
        .filter_map(|(p, n)| from_sysfs(p, n))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn write_file(p: &Path, s: &str) {
        let mut f = fs::File::create(p).unwrap();
        f.write_all(s.as_bytes()).unwrap();
    }

    fn write_router(base: &Path, route: &str, generation: u8, extras: &[(&str, &str)]) {
        let dir = base.join(route);
        fs::create_dir_all(&dir).unwrap();
        write_file(&dir.join("generation"), &generation.to_string());
        for (k, v) in extras {
            write_file(&dir.join(k), v);
        }
    }

    #[test]
    fn missing_thunderbolt_dir_returns_empty() {
        assert!(enumerate_in(Path::new("/no/such/usbeehive/path")).is_empty());
    }

    #[test]
    fn enumerates_host_and_device_routers() {
        let d = super::super::reader::tempdir_lite::TmpDir::new("tb-enum");
        write_router(
            d.path(),
            "0-0",
            4,
            &[("vendor_name", "Intel"), ("device_name", "Maple Ridge")],
        );
        write_router(
            d.path(),
            "0-1",
            4,
            &[("vendor_name", "OWC"), ("unique_id", "abcd")],
        );
        write_router(d.path(), "0-101", 3, &[]);
        // Subsystem link without a generation file → ignored.
        fs::create_dir_all(d.path().join("domain0")).unwrap();

        let routers = enumerate_in(d.path());
        assert_eq!(routers.len(), 3);

        let host = routers.iter().find(|r| r.route == "0-0").unwrap();
        assert!(host.is_host);
        assert_eq!(host.generation, 4);
        assert_eq!(host.vendor_name, "Intel");

        let dev = routers.iter().find(|r| r.route == "0-1").unwrap();
        assert!(!dev.is_host);
        assert_eq!(dev.unique_id, "abcd");

        let nested = routers.iter().find(|r| r.route == "0-101").unwrap();
        assert!(!nested.is_host);
        assert_eq!(nested.generation, 3);
    }

    #[test]
    fn routers_without_generation_are_retained_as_unknown() {
        let d = super::super::reader::tempdir_lite::TmpDir::new("tb-no-gen");
        let router = d.path().join("0-0");
        fs::create_dir_all(&router).unwrap();
        // Router identity attributes distinguish this from a route-shaped
        // service entry even though the generation attribute is unavailable.
        write_file(&router.join("vendor"), "0x8086\n");
        write_file(&router.join("device"), "0x1234\n");
        // No `generation` file written.
        let routers = enumerate_in(d.path());
        assert_eq!(routers.len(), 1);
        assert_eq!(routers[0].reported_generation, None);
        assert_eq!(routers[0].generation, 0);
    }

    #[test]
    fn route_shaped_service_without_router_identity_is_ignored() {
        let d = super::super::reader::tempdir_lite::TmpDir::new("tb-service-name");
        let service = d.path().join("0-1");
        fs::create_dir_all(&service).unwrap();
        write_file(&service.join("service_name"), "dma_test\n");

        assert!(enumerate_in(d.path()).is_empty());
    }
}
