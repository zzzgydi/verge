//! Download one official release asset with its published SHA-256 checksum.
//! A moving `latest` release can cause a safe digest mismatch, never unchecked data.
use super::*;
use crate::domain::GeoDataKind;
use sha2::{Digest, Sha256};

pub fn geo_asset(kind: GeoDataKind, directory: &Path) -> Result<(&'static str, PathBuf), AppError> {
    let (asset, default, aliases): (&str, &str, &[&str]) = match kind {
        GeoDataKind::GeoIp => ("geoip.dat", "GeoIP.dat", &["geoip.dat"]),
        GeoDataKind::GeoSite => ("geosite.dat", "GeoSite.dat", &["geosite.dat"]),
        GeoDataKind::Country => (
            "geoip.metadb",
            "geoip.metadb",
            &["country.mmdb", "geoip.db", "geoip.metadb"],
        ),
    };
    let mut entries: Vec<_> = fs::read_dir(directory)
        .map_err(profile_fetch_error)?
        .collect::<Result<_, _>>()
        .map_err(profile_fetch_error)?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if aliases.contains(&name.as_str()) {
            if !entry.file_type().map_err(profile_fetch_error)?.is_file() {
                return Err(AppError::new(
                    ErrorCode::ValidationFailed,
                    "Geo data target must be a regular file",
                ));
            }
            let asset = if kind == GeoDataKind::Country && name != "geoip.metadb" {
                "country.mmdb"
            } else {
                asset
            };
            return Ok((asset, entry.path()));
        }
    }
    Ok((asset, directory.join(default)))
}

pub fn download_geo_candidate<F: ArtifactFetcher>(
    fetcher: &mut F,
    asset: &str,
    staging: &Path,
) -> Result<(PathBuf, String), AppError> {
    if !["geoip.dat", "geosite.dat", "country.mmdb", "geoip.metadb"].contains(&asset) {
        return Err(AppError::new(ErrorCode::InvalidInput, "unknown Geo asset"));
    }
    let url =
        format!("https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/{asset}");
    let checksum = fetcher.fetch(&format!("{url}.sha256sum"))?;
    if checksum.len() > 4096 {
        return Err(AppError::new(
            ErrorCode::ValidationFailed,
            "Geo checksum file is too large",
        ));
    }
    let checksum = std::str::from_utf8(&checksum)
        .map_err(|_| AppError::new(ErrorCode::ValidationFailed, "invalid Geo checksum file"))?;
    let fields: Vec<_> = checksum.split_whitespace().collect();
    if fields.len() != 2
        || fields[0].len() != 64
        || !fields[0].bytes().all(|b| b.is_ascii_hexdigit())
        || fields[1].trim_start_matches('*') != asset
    {
        return Err(AppError::new(
            ErrorCode::ValidationFailed,
            "invalid SHA-256 entry for Geo asset",
        ));
    }
    let digest = fields[0].to_ascii_lowercase();
    let bytes = fetcher.fetch(&url)?;
    if bytes.is_empty()
        || bytes.len() > 64 * 1024 * 1024
        || format!("{:x}", Sha256::digest(&bytes)) != digest
    {
        return Err(AppError::new(
            ErrorCode::ValidationFailed,
            "Geo release checksum mismatch; retry the update",
        ));
    }
    fs::create_dir_all(staging).map_err(profile_fetch_error)?;
    let candidate = staging.join(asset);
    fs::write(&candidate, bytes).map_err(profile_fetch_error)?;
    Ok((candidate, digest))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fetcher(Vec<Vec<u8>>);
    impl ArtifactFetcher for Fetcher {
        fn fetch(&mut self, _: &str) -> Result<Vec<u8>, AppError> {
            Ok(self.0.remove(0))
        }
    }
    #[test]
    fn bad_geo_download_never_creates_a_candidate() {
        let directory = std::env::temp_dir().join(format!("verge-geodata-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let body = b"valid".to_vec();
        let metadata = format!("{:x}  geosite.dat\n", Sha256::digest(&body)).into_bytes();
        let mut failed = Fetcher(vec![metadata.clone(), b"wrong".to_vec()]);
        assert!(download_geo_candidate(&mut failed, "geosite.dat", &directory).is_err());
        assert!(!directory.exists());
        let (candidate, _) = download_geo_candidate(
            &mut Fetcher(vec![metadata, body.clone()]),
            "geosite.dat",
            &directory,
        )
        .unwrap();
        assert_eq!(fs::read(candidate).unwrap(), body);
        fs::write(directory.join("Country.mmdb"), b"old").unwrap();
        let (asset, target) = geo_asset(GeoDataKind::Country, &directory).unwrap();
        assert_eq!(asset, "country.mmdb");
        assert_eq!(target.file_name().unwrap(), "Country.mmdb");
        fs::remove_dir_all(directory).unwrap();
    }
}
