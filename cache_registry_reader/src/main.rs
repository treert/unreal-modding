use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::time::{SystemTime, UNIX_EPOCH};

use byteorder::{ReadBytesExt, LE};
use serde::Serialize;
use serde_json;

use unreal_asset_base::containers::{Chain, NameMap};
use unreal_asset_base::object_version::{ObjectVersion, ObjectVersionUE5};
use unreal_asset_base::reader::{ArchiveReader, ArchiveTrait, RawReader};
use unreal_asset_base::types::PackageIndex;

// ---------------------------------------------------------------------------
// Type aliases
// ---------------------------------------------------------------------------

type CursorType = Cursor<Vec<u8>>;
type Reader = RawReader<PackageIndex, CursorType>;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAGIC: u32 = 0x9E2A83C1;
const EXPECTED_HEADER_VERSION: i32 = 15;
const ASSET_REGISTRY_GUID: [u32; 4] = [0x717F9EE7, 0xE9B0493A, 0x88B39132, 0x1B388107];
const EXPECTED_ASSET_REGISTRY_VERSION: i32 = 7;

const VERSION: &str = env!("CARGO_PKG_VERSION");

// ---------------------------------------------------------------------------
// Filter configuration
// ---------------------------------------------------------------------------

#[derive(Default)]
struct FilterConfig {
    class_set: HashSet<String>, // exact match, case-insensitive, OR within class
    path_glob: Option<String>,  // glob pattern for package_name, case-insensitive
    chunk_set: HashSet<i32>,    // chunk_ids contains any of these
}

impl FilterConfig {
    fn is_active(&self) -> bool {
        !self.class_set.is_empty() || self.path_glob.is_some() || !self.chunk_set.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Case-insensitive glob matching (supports * and ?)
// ---------------------------------------------------------------------------

fn glob_match(pattern: &str, text: &str) -> bool {
    glob_match_impl(&pattern.to_lowercase(), &text.to_lowercase())
}

fn glob_match_impl(p: &str, t: &str) -> bool {
    let pb = p.as_bytes();
    let tb = t.as_bytes();
    let mut pi = 0;
    let mut ti = 0;
    let mut star_p = None;
    let mut match_t = 0;

    while ti < tb.len() || pi < pb.len() {
        if pi < pb.len() && pb[pi] == b'*' {
            star_p = Some(pi);
            match_t = ti;
            pi += 1;
        } else if pi < pb.len() && ti < tb.len() && (pb[pi] == b'?' || pb[pi] == tb[ti]) {
            pi += 1;
            ti += 1;
        } else if let Some(sp) = star_p {
            pi = sp + 1;
            match_t += 1;
            ti = match_t;
        } else {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// JSON output structures (matching DevelopmentAssetRegistry.bin format)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct OutputMetadata {
    #[serde(rename = "ExportTime")]
    export_time: String,
    #[serde(rename = "EngineVersion")]
    engine_version: String,
    #[serde(rename = "Tool")]
    tool: String,
    #[serde(rename = "Version")]
    version: String,
    #[serde(rename = "IncludeHardReferences")]
    include_hard_references: bool,
    #[serde(rename = "IncludeSoftReferences")]
    include_soft_references: bool,
    #[serde(rename = "IncludeMetadata")]
    include_metadata: bool,
    #[serde(rename = "TotalAssets")]
    total_assets: usize,
}

#[derive(Serialize)]
struct DepsContainer {
    #[serde(rename = "Hard")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    hard: Vec<String>,
    #[serde(rename = "Soft")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    soft: Vec<String>,
}

#[derive(Serialize)]
struct AssetEntry {
    #[serde(rename = "ObjectPath")]
    object_path: String,
    #[serde(rename = "PackageName")]
    package_name: String,
    #[serde(rename = "AssetName")]
    asset_name: String,
    #[serde(rename = "AssetClass")]
    asset_class: String,
    #[serde(rename = "PackagePath")]
    package_path: String,
    #[serde(rename = "PackageGuid")]
    package_guid: String,
    #[serde(rename = "ChunkIDs")]
    chunk_ids: Vec<i32>,
    #[serde(rename = "DirectDependencies")]
    direct_dependencies: DepsContainer,
    #[serde(rename = "DependencyCount")]
    dependency_count: usize,
}

#[derive(Serialize)]
struct Output {
    #[serde(rename = "Metadata")]
    metadata: OutputMetadata,
    #[serde(rename = "Assets")]
    assets: Vec<AssetEntry>,
}

// ---------------------------------------------------------------------------
// FName / FString helpers
// ---------------------------------------------------------------------------

fn read_fname_str(reader: &mut Reader) -> Result<String, Box<dyn std::error::Error>> {
    let fname = reader.read_fname()?;
    Ok(fname.get_content(|s| s.to_string()))
}

fn read_fstring_unlimited(
    reader: &mut Reader,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    use std::mem::size_of;
    let len: i32 = reader.read_i32::<LE>()?;
    if len == 0 {
        return Ok(None);
    }
    let (len, is_wide) = if len < 0 {
        (-len as usize, true)
    } else {
        (len as usize, false)
    };
    if is_wide {
        let byte_len = len.saturating_sub(1) * size_of::<u16>();
        let mut buf = vec![0u8; byte_len];
        reader.read_exact(&mut buf)?;
        reader.read_u16::<LE>()?;
        let u16_data: Vec<u16> = buf
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        Ok(Some(String::from_utf16(&u16_data)?))
    } else {
        let byte_len = len.saturating_sub(1);
        let mut buf = vec![0u8; byte_len];
        reader.read_exact(&mut buf)?;
        reader.read_u8()?;
        Ok(Some(String::from_utf8(buf)?))
    }
}

fn skip_fname(reader: &mut Reader) -> Result<(), Box<dyn std::error::Error>> {
    reader.read_i32::<LE>()?;
    reader.read_i32::<LE>()?;
    Ok(())
}

fn read_guid_str(reader: &mut Reader) -> Result<String, Box<dyn std::error::Error>> {
    let a = reader.read_u32::<LE>()?;
    let b = reader.read_u32::<LE>()?;
    let c = reader.read_u32::<LE>()?;
    let d = reader.read_u32::<LE>()?;
    Ok(format!("{{{:08X}-{:08X}-{:08X}-{:08X}}}", a, b, c, d))
}

fn skip_bitarray(reader: &mut Reader) -> Result<(), Box<dyn std::error::Error>> {
    let num_bits = reader.read_i32::<LE>()?;
    let num_words = (num_bits + 31) / 32;
    for _ in 0..num_words {
        reader.read_u32::<LE>()?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Name table loading
// ---------------------------------------------------------------------------

fn load_name_table(
    reader: &mut Reader,
    name_table_offset: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    reader.seek(SeekFrom::Start(name_table_offset as u64))?;
    let name_count = reader.read_i32::<LE>()?;
    println!("  Name table: {} entries", name_count);
    for i in 0..name_count {
        let name = reader.read_fstring()?.unwrap_or_default();
        reader
            .get_name_map()
            .get_mut()
            .add_name_reference(name, false);
        reader.read_u16::<LE>()?; // NonCasePreservingHash
        reader.read_u16::<LE>()?; // CasePreservingHash
        if i < 5 {
            let nm = reader.get_name_map();
            let r = nm.get_ref();
            let list = r.get_name_map_index_list();
            println!(
                "    [{}] {}",
                list.len() - 1,
                r.get_name_reference((list.len() - 1) as i32)
            );
        }
    }
    println!("    ... {} total names loaded", name_count);
    Ok(())
}

// ---------------------------------------------------------------------------
// Core asset + dependency parsing
// ---------------------------------------------------------------------------

struct AssetCore {
    object_path: String,
    package_path: String,
    asset_class: String,
    package_name: String,
    asset_name: String,
    chunk_ids: Vec<i32>,
    #[allow(dead_code)]
    tags_and_values: Vec<(String, String)>,
}

fn parse_asset_core(
    reader: &mut Reader,
    include_metadata: bool,
) -> Result<AssetCore, Box<dyn std::error::Error>> {
    let object_path = read_fname_str(reader)?;
    let package_path = read_fname_str(reader)?;
    let asset_class = read_fname_str(reader)?;
    let package_name = read_fname_str(reader)?;
    let asset_name = read_fname_str(reader)?;

    // TagsAndValues (TMap<FName, FString>)
    let tag_count = reader.read_i32::<LE>()?;
    let tags = if include_metadata && tag_count > 0 {
        let mut tags = Vec::with_capacity(tag_count as usize);
        for _ in 0..tag_count {
            let key = read_fname_str(reader)?;
            let val = read_fstring_unlimited(reader)?.unwrap_or_default();
            tags.push((key, val));
        }
        tags
    } else {
        for _ in 0..tag_count {
            read_fname_str(reader)?;
            read_fstring_unlimited(reader)?;
        }
        Vec::new()
    };

    // ChunkIDs (TArray<int32>)
    let chunk_count = reader.read_i32::<LE>()?;
    let mut chunk_ids = Vec::with_capacity(chunk_count as usize);
    for _ in 0..chunk_count {
        chunk_ids.push(reader.read_i32::<LE>()?);
    }

    // Skip PackageFlags (uint32)
    reader.read_u32::<LE>()?;

    Ok(AssetCore {
        object_path,
        package_path,
        asset_class,
        package_name,
        asset_name,
        chunk_ids,
        tags_and_values: tags,
    })
}

struct PackageDepData {
    package_guid: String,
    hard_deps: Vec<String>,
    soft_deps: Vec<String>,
}

fn parse_dependency_data(
    reader: &mut Reader,
) -> Result<PackageDepData, Box<dyn std::error::Error>> {
    // PackageName (FName) — skip, we already have it
    skip_fname(reader)?;

    // TArray<FObjectImport> ImportMap → collect package names for hard deps
    let import_count = reader.read_i32::<LE>()?;
    let mut hard_deps = Vec::with_capacity(import_count as usize);
    for _ in 0..import_count {
        skip_fname(reader)?; // ClassPackage
        skip_fname(reader)?; // ClassName
        reader.read_i32::<LE>()?; // OuterIndex
        skip_fname(reader)?; // ObjectName
        let dep_name = read_fname_str(reader)?;
        if dep_name != "None" {
            hard_deps.push(dep_name);
        }
    }
    hard_deps.sort();
    hard_deps.dedup();

    // TArray<FName> SoftPackageReferenceList
    let soft_ref_count = reader.read_i32::<LE>()?;
    let mut soft_deps = Vec::with_capacity(soft_ref_count as usize);
    for _ in 0..soft_ref_count {
        let dep = read_fname_str(reader)?;
        if dep != "None" {
            soft_deps.push(dep);
        }
    }
    soft_deps.sort();
    soft_deps.dedup();

    // TMap<FPackageIndex, TArray<FName>> SearchableNamesMap — skip
    let map_count = reader.read_i32::<LE>()?;
    for _ in 0..map_count {
        reader.read_i32::<LE>()?; // key
        let name_count = reader.read_i32::<LE>()?;
        for _ in 0..name_count {
            skip_fname(reader)?;
        }
    }

    // FAssetPackageData::SerializeForCache
    skip_fname(reader)?; // PackageName
    let package_guid = read_guid_str(reader)?; // Guid (16 bytes)
    reader.read_i64::<LE>()?; // skip trailing 8 bytes (zeros in editor cache)

    // TBitArray<> ImportUsedInGame
    skip_bitarray(reader)?;
    // TBitArray<> SoftPackageUsedInGame
    skip_bitarray(reader)?;

    Ok(PackageDepData {
        package_guid,
        hard_deps,
        soft_deps,
    })
}

// ---------------------------------------------------------------------------
// Package parsing (flattened to asset list)
// ---------------------------------------------------------------------------

fn filter_asset(asset: &AssetCore, filter: &FilterConfig) -> bool {
    // class filter (OR within dimension)
    if !filter.class_set.is_empty() {
        let class_lower = asset.asset_class.to_lowercase();
        if !filter
            .class_set
            .iter()
            .any(|c| c.to_lowercase() == class_lower)
        {
            return false;
        }
    }
    // path filter (glob against package_name — which is path + name)
    if let Some(ref pat) = filter.path_glob {
        if !glob_match(pat, &asset.package_name) {
            return false;
        }
    }
    // chunk filter (OR within dimension)
    if !filter.chunk_set.is_empty() {
        if !asset
            .chunk_ids
            .iter()
            .any(|id| filter.chunk_set.contains(id))
        {
            return false;
        }
    }
    true
}

fn parse_all_assets(
    reader: &mut Reader,
    num_packages: i32,
    limit: Option<usize>,
    no_hard_refs: bool,
    no_soft_refs: bool,
    include_metadata: bool,
    filter: &FilterConfig,
) -> Result<Vec<AssetEntry>, Box<dyn std::error::Error>> {
    let actual_pkg_count = if let Some(lim) = limit {
        num_packages.min(lim as i32) as usize
    } else {
        num_packages as usize
    };

    let mut assets = Vec::new();
    let show_progress = !filter.is_active();

    for i in 0..actual_pkg_count {
        // FDiskCachedAssetData
        read_fname_str(reader)?; // PackageName (outer) — skip
        reader.read_i64::<LE>()?; // Timestamp — skip
        read_fname_str(reader)?; // Extension — skip
        let asset_data_count = reader.read_i32::<LE>()?;

        // Collect core asset data
        let mut asset_cores = Vec::with_capacity(asset_data_count as usize);
        for _ in 0..asset_data_count {
            asset_cores.push(parse_asset_core(reader, include_metadata)?);
        }

        // Parse dependency data (shared by all assets in this package)
        let dep = parse_dependency_data(reader)?;

        // Emit flattened entries (with optional filtering)
        for a in asset_cores {
            if !filter_asset(&a, filter) {
                continue;
            }
            let hard = if no_hard_refs {
                Vec::new()
            } else {
                dep.hard_deps.clone()
            };
            let soft = if no_soft_refs {
                Vec::new()
            } else {
                dep.soft_deps.clone()
            };
            let dep_count = hard.len() + soft.len();
            assets.push(AssetEntry {
                object_path: a.object_path,
                package_name: a.package_name,
                asset_name: a.asset_name,
                asset_class: a.asset_class,
                package_path: a.package_path,
                package_guid: dep.package_guid.clone(),
                chunk_ids: a.chunk_ids,
                direct_dependencies: DepsContainer { hard, soft },
                dependency_count: dep_count,
            });
        }

        if show_progress && (i + 1) % 10000 == 0 {
            println!(
                "  Parsed {}/{} packages, {} assets...",
                i + 1,
                actual_pkg_count,
                assets.len()
            );
        }
    }

    Ok(assets)
}

// ---------------------------------------------------------------------------
// CachedAssetRegistry.bin parsing (extracted from main)
// ---------------------------------------------------------------------------

fn parse_cached_registry(
    reader: &mut Reader,
    limit: Option<usize>,
    no_hard_refs: bool,
    no_soft_refs: bool,
    include_metadata: bool,
    filter: &FilterConfig,
) -> Result<(Vec<AssetEntry>, String), Box<dyn std::error::Error>> {
    // ---- FNameTableArchive Header ----
    let magic = reader.read_u32::<LE>()?;
    if magic != MAGIC {
        return Err(format!("Bad magic: 0x{:08X}, expected 0x{:08X}", magic, MAGIC).into());
    }
    println!("Magic: 0x{:08X} OK", magic);

    let header_version = reader.read_i32::<LE>()?;
    if header_version != EXPECTED_HEADER_VERSION {
        eprintln!(
            "WARNING: Header version = {}, expected {}",
            header_version, EXPECTED_HEADER_VERSION
        );
    } else {
        println!("Header Version: {} OK", header_version);
    }

    let name_table_offset = reader.read_i64::<LE>()?;
    println!("NameTableOffset: 0x{:X}", name_table_offset);

    // ---- Load name table ----
    println!("\n=== Loading Name Table ===");
    load_name_table(reader, name_table_offset)?;

    // ---- Seek back to body start ----
    reader.seek(SeekFrom::Start(0x10))?;

    // ---- Validate AssetRegistryVersion ----
    let guid_a = reader.read_u32::<LE>()?;
    let guid_b = reader.read_u32::<LE>()?;
    let guid_c = reader.read_u32::<LE>()?;
    let guid_d = reader.read_u32::<LE>()?;
    if guid_a != ASSET_REGISTRY_GUID[0]
        || guid_b != ASSET_REGISTRY_GUID[1]
        || guid_c != ASSET_REGISTRY_GUID[2]
        || guid_d != ASSET_REGISTRY_GUID[3]
    {
        eprintln!(
            "WARNING: AssetRegistryVersion Guid mismatch: got {:08X}-{:08X}-{:08X}-{:08X}, expected {:08X}-{:08X}-{:08X}-{:08X}",
            guid_a, guid_b, guid_c, guid_d,
            ASSET_REGISTRY_GUID[0], ASSET_REGISTRY_GUID[1], ASSET_REGISTRY_GUID[2], ASSET_REGISTRY_GUID[3],
        );
    } else {
        println!("AssetRegistryVersion Guid: OK");
    }

    let asset_registry_version = reader.read_i32::<LE>()?;
    if asset_registry_version != EXPECTED_ASSET_REGISTRY_VERSION {
        eprintln!(
            "WARNING: AssetRegistryVersion = {}, expected {}",
            asset_registry_version, EXPECTED_ASSET_REGISTRY_VERSION
        );
    } else {
        println!("AssetRegistryVersion: {} OK", asset_registry_version);
    }

    // ---- Parse body ----
    let num_packages = reader.read_i32::<LE>()?;
    println!("\n=== Parsing Packages ===");
    println!("NumPackages: {}", num_packages);
    if let Some(lim) = limit {
        println!("Limit: first {} packages", lim);
    }

    let assets = parse_all_assets(
        reader,
        num_packages,
        limit,
        no_hard_refs,
        no_soft_refs,
        include_metadata,
        filter,
    )?;

    if filter.is_active() {
        println!("  Filter matched {} assets", assets.len());
    }

    Ok((assets, String::from("4.26.2")))
}

// ---------------------------------------------------------------------------
// DevelopmentAssetRegistry.bin parsing
// ---------------------------------------------------------------------------

/// Parse a DevelopmentAssetRegistry.bin (cooked FAssetRegistryState).
///
/// Format:
///   GUID(16) + Version(4) + NameTableOffset(8) + Body(flat FAssetData list + Deps + PackageData)
///
/// The FAssetData::SerializeForCache format is identical to the one used in
/// CachedAssetRegistry.bin, so we reuse `parse_asset_core`.
fn parse_dev_registry(
    reader: &mut Reader,
    limit: Option<usize>,
    include_metadata: bool,
    filter: &FilterConfig,
) -> Result<(Vec<AssetEntry>, i32), Box<dyn std::error::Error>> {
    // ---- Header: GUID + Version ----
    let guid_a = reader.read_u32::<LE>()?;
    let guid_b = reader.read_u32::<LE>()?;
    let guid_c = reader.read_u32::<LE>()?;
    let guid_d = reader.read_u32::<LE>()?;
    if guid_a != ASSET_REGISTRY_GUID[0]
        || guid_b != ASSET_REGISTRY_GUID[1]
        || guid_c != ASSET_REGISTRY_GUID[2]
        || guid_d != ASSET_REGISTRY_GUID[3]
    {
        eprintln!(
            "WARNING: AssetRegistryVersion Guid mismatch: got {:08X}-{:08X}-{:08X}-{:08X}",
            guid_a, guid_b, guid_c, guid_d,
        );
    } else {
        println!("AssetRegistryVersion Guid: OK");
    }

    let version = reader.read_i32::<LE>()?;
    println!(
        "AssetRegistryVersion: {} ({})",
        version,
        version_name(version)
    );

    if version < 4 {
        return Err(format!("Unsupported AssetRegistryVersion: {} (min: 4)", version).into());
    }
    if version > 8 {
        return Err(format!(
            "Unsupported AssetRegistryVersion: {} (max supported: 8, LetsGo AddedReCookFlags)",
            version
        )
        .into());
    }

    // ---- NameTableOffset ----
    let name_table_offset = reader.read_i64::<LE>()?;
    println!("NameTableOffset: 0x{:X}", name_table_offset);

    // ---- Load name table ----
    println!("\n=== Loading Name Table ===");
    load_name_table(reader, name_table_offset)?;

    // ---- Seek back to body start (0x1C) ----
    reader.seek(SeekFrom::Start(0x1C))?;

    // ---- Asset count ----
    let asset_count = reader.read_i32::<LE>()?;
    println!("\n=== Parsing Assets ===");
    println!("AssetCount: {}", asset_count);
    if let Some(lim) = limit {
        println!("Limit: first {} assets", lim);
    }

    let actual_count = if let Some(lim) = limit {
        (asset_count as usize).min(lim)
    } else {
        asset_count as usize
    };
    let show_progress = !filter.is_active();

    // Parse assets we care about
    let mut asset_cores = Vec::with_capacity(actual_count);
    for i in 0..actual_count {
        asset_cores.push(parse_asset_core(reader, include_metadata)?);
        if show_progress && (i + 1) % 50000 == 0 {
            println!("  Parsed {}/{} assets (collected)...", i + 1, actual_count);
        }
    }

    // Skip remaining assets if limit is set (must read through to reach dep section)
    let total_to_read = asset_count as usize;
    if actual_count < total_to_read {
        let remaining = total_to_read - actual_count;
        println!(
            "  Skipping remaining {} assets to reach dependency section...",
            remaining
        );
        for i in 0..remaining {
            parse_asset_core(reader, false)?; // skip without metadata collection
            if (i + 1) % 50000 == 0 {
                println!("  Skipped {}/{} assets...", i + 1, remaining);
            }
        }
    }

    // ---- Dependency section ----
    println!("\n=== Dependency Section ===");
    let mut guid_map: HashMap<String, String> = HashMap::new();

    if version >= 7 {
        // AddedDependencyFlags: section is wrapped with a size field.
        let dep_section_size = reader.read_i64::<LE>()?;
        println!(
            "DependencySectionSize: {} bytes ({:.2} MB)",
            dep_section_size,
            dep_section_size as f64 / 1024.0 / 1024.0
        );
        println!("  Skipping dependency section (DependsNode graph not yet parsed)");
        reader.seek(SeekFrom::Current(dep_section_size))?;
    } else {
        eprintln!("WARNING: Version {} < 7, DependsNode skipping not implemented. Dependencies will be empty.", version);
        // For old versions, we can't easily skip the dependency section.
        // Just warn and continue; PackageData parsing may fail.
    }

    // ---- Package data ----
    println!("\n=== Package Data ===");
    let num_package_data = reader.read_i32::<LE>()?;
    println!("PackageData count: {}", num_package_data);

    // NOTE: Format confirmed by UE source (AssetRegistryState.cpp:2586-2589, Archive.cpp:482-505):
    //   FName(8) + DiskSize(8) + Guid(16) + bValid(4) + [hash(16 if bValid!=0)] + ReCook(4)
    // UE serializes bool through FArchive as legacy UBOOL (uint32), not 1 byte.
    // Entries are variable-length: 40 bytes (bValid=0) or 56 bytes (bValid=1).
    for i in 0..num_package_data {
        let (pkg_name, guid) = read_dev_package_data_entry(reader, version)?;
        guid_map.insert(pkg_name, guid);
        if (i + 1) % 50000 == 0 {
            println!(
                "  Parsed {}/{} package data entries...",
                i + 1,
                num_package_data
            );
        }
    }

    // ---- Build output ----
    let mut assets = Vec::with_capacity(asset_cores.len());
    for a in asset_cores {
        if !filter_asset(&a, filter) {
            continue;
        }
        let pkg_guid = guid_map.get(&a.package_name).cloned().unwrap_or_default();
        assets.push(AssetEntry {
            object_path: a.object_path,
            package_name: a.package_name,
            asset_name: a.asset_name,
            asset_class: a.asset_class,
            package_path: a.package_path,
            package_guid: pkg_guid,
            chunk_ids: a.chunk_ids,
            direct_dependencies: DepsContainer {
                hard: Vec::new(),
                soft: Vec::new(),
            },
            dependency_count: 0,
        });
    }

    if filter.is_active() {
        println!("  Filter matched {} assets", assets.len());
    }

    Ok((assets, version))
}

/// Read one FAssetPackageData entry from DevelopmentAssetRegistry.bin.
/// Returns (package_name, package_guid).
fn read_dev_package_data_entry(
    reader: &mut Reader,
    version: i32,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let pkg_name = read_fname_str(reader)?; // PackageName (8 bytes)
    reader.read_i64::<LE>()?; // DiskSize (8 bytes)
    let guid = read_guid_str(reader)?; // Guid (16 bytes)

    // FArchive serializes bool as legacy UBOOL (uint32), so FMD5Hash::bIsValid is 4 bytes.
    if version >= 6 {
        let b_is_valid = reader.read_u32::<LE>()?;
        if b_is_valid != 0 {
            let mut hash_buf = [0u8; 16];
            reader.read_exact(&mut hash_buf)?;
        }
    }

    // ReCook flag (LetsGo custom bool, if version >= AddedReCookFlags == 8)
    if version >= 8 {
        reader.read_u32::<LE>()?;
    }

    Ok((pkg_name, guid))
}

/// Human-readable name for an FAssetRegistryVersion value.
fn version_name(v: i32) -> &'static str {
    match v {
        0 => "PreVersioning",
        1 => "HardSoftDependencies",
        2 => "AddAssetRegistryState",
        3 => "ChangedAssetData",
        4 => "RemovedMD5Hash",
        5 => "AddedHardManage",
        6 => "AddedCookedMD5Hash",
        7 => "AddedDependencyFlags",
        8 => "AddedReCookFlags (LetsGo)",
        _ => "Unknown",
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "Usage: {} <RegistryFile.bin> [./tmp/<input-file-name>.json] [Options]",
            args[0]
        );
        eprintln!();
        eprintln!("  Parses UE4 CachedAssetRegistry.bin or DevelopmentAssetRegistry.bin");
        eprintln!("  and exports to JSON. Format is auto-detected.");
        eprintln!("  Default output: ./tmp/<input-file-name>.json (e.g. foo.bin -> ./tmp/foo.bin.json)");
        eprintln!();
        eprintln!("Options:");
        eprintln!("  --limit N         Only export the first N packages/assets (default: all)");
        eprintln!("  -NoHardRefs       Exclude hard references");
        eprintln!("  -NoSoftRefs       Exclude soft references");
        eprintln!("  -IncludeMetadata  Include asset metadata (tags & values)");
        eprintln!();
        eprintln!("Filter Options (case-insensitive, combined with AND):");
        eprintln!("  --class A,B,...   Filter by asset class (exact match, OR within)");
        eprintln!("  --path GLOB       Filter by package name (glob: * = any, ? = one)");
        eprintln!("  --chunk N,M,...   Filter by chunk ID (contains any, OR within)");
        eprintln!();
        eprintln!("Examples:");
        eprintln!("  --class Blueprint,Texture2D");
        eprintln!("  --path /Game/Characters/*");
        eprintln!("  --path */BP_Sword*            (filter by asset name)");
        eprintln!("  --class SkeletalMesh --path /Game/Characters/*/BP_Sword*");
        return Ok(());
    }

    let input_path = &args[1];
    let input_file_name = std::path::Path::new(input_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let mut output_path = format!("./tmp/{}.json", input_file_name);
    let mut limit: Option<usize> = None;
    let mut no_hard_refs = false;
    let mut no_soft_refs = false;
    let mut include_metadata = false;
    let mut filter = FilterConfig::default();
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--limit" => {
                i += 1;
                if i < args.len() {
                    limit = Some(args[i].parse()?);
                }
            }
            "-NoHardRefs" => no_hard_refs = true,
            "-NoSoftRefs" => no_soft_refs = true,
            "-IncludeMetadata" => include_metadata = true,
            "--class" => {
                i += 1;
                if i < args.len() {
                    for c in args[i].split(',') {
                        let trimmed = c.trim();
                        if !trimmed.is_empty() {
                            filter.class_set.insert(trimmed.to_string());
                        }
                    }
                }
            }
            "--path" => {
                i += 1;
                if i < args.len() {
                    filter.path_glob = Some(args[i].clone());
                }
            }
            "--chunk" => {
                i += 1;
                if i < args.len() {
                    for n in args[i].split(',') {
                        if let Ok(id) = n.trim().parse::<i32>() {
                            filter.chunk_set.insert(id);
                        }
                    }
                }
            }
            other => {
                output_path = other.to_string();
            }
        }
        i += 1;
    }

    // ---- Read file ----
    println!("Loading: {}", input_path);
    let file_bytes = fs::read(input_path)?;
    let file_len = file_bytes.len();
    println!(
        "File size: {} bytes ({:.2} MB)",
        file_len,
        file_len as f64 / 1024.0 / 1024.0
    );

    // ---- Detect format by first 4 bytes ----
    let first_u32 = {
        let mut c = Cursor::new(&file_bytes);
        c.read_u32::<LE>()?
    };

    let is_cached = first_u32 == MAGIC;
    let is_dev = first_u32 == ASSET_REGISTRY_GUID[0];

    if !is_cached && !is_dev {
        eprintln!(
            "ERROR: Unknown file format. First 4 bytes: 0x{:08X} (expected 0x{:08X} for Cached or 0x{:08X} for Development)",
            first_u32, MAGIC, ASSET_REGISTRY_GUID[0]
        );
        return Err("Unknown file format".into());
    }

    println!(
        "Format detected: {}",
        if is_cached {
            "CachedAssetRegistry.bin"
        } else {
            "DevelopmentAssetRegistry.bin"
        }
    );

    // ---- Create reader (shared setup) ----
    let cursor = Cursor::new(file_bytes);
    let name_map = NameMap::new();
    let mut reader = RawReader::new(
        Chain::new(cursor, None),
        ObjectVersion::VER_UE4_ASSETREGISTRY_DEPENDENCYFLAGS,
        ObjectVersionUE5::UNKNOWN,
        false,
        name_map.clone(),
    );

    // ---- Parse based on format ----
    let (assets, engine_version) = if is_cached {
        parse_cached_registry(
            &mut reader,
            limit,
            no_hard_refs,
            no_soft_refs,
            include_metadata,
            &filter,
        )?
    } else {
        let (assets, version) = parse_dev_registry(&mut reader, limit, include_metadata, &filter)?;
        let ev = format!("4.26.2 (registry v{})", version);
        (assets, ev)
    };

    // ---- Build metadata ----
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let now_secs = now.as_secs();
    let days_since_epoch = now_secs / 86400;
    let (year, month, day) = civil_from_days(days_since_epoch as i64);
    let time_of_day = now_secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;
    let export_time = format!(
        "{:04}.{:02}.{:02}-{:02}.{:02}.{:02}",
        year, month, day, hours, minutes, seconds
    );

    let total_assets = assets.len();

    // ---- Write JSON ----
    println!("\n=== Writing JSON ({} assets) ===", total_assets);
    let output = Output {
        metadata: OutputMetadata {
            export_time,
            engine_version,
            tool: String::from("cache-registry-reader"),
            version: VERSION.to_string(),
            include_hard_references: !no_hard_refs,
            include_soft_references: !no_soft_refs,
            include_metadata,
            total_assets,
        },
        assets,
    };

    let json = serde_json::to_string_pretty(&output)?;
    // Ensure output directory exists
    if let Some(parent) = std::path::Path::new(&output_path).parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output_path, json)?;

    let output_size = fs::metadata(&output_path)?.len();
    println!(
        "Output: {} ({:.2} MB)",
        output_path,
        output_size as f64 / 1024.0 / 1024.0
    );
    println!("Done!");

    Ok(())
}

// ---------------------------------------------------------------------------
// Simple date calculation (no extra deps)
// ---------------------------------------------------------------------------

/// Convert days since Unix epoch to (year, month, day).
/// Algorithm from http://howardhinnant.github.io/date_algorithms.html
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::WriteBytesExt;

    fn package_data_reader(bytes: Vec<u8>) -> Reader {
        let mut name_map = NameMap::new();
        name_map
            .get_mut()
            .add_name_reference("/Game/TestPackage".to_string(), false);

        RawReader::new(
            Chain::new(Cursor::new(bytes), None),
            ObjectVersion::VER_UE4_ASSETREGISTRY_DEPENDENCYFLAGS,
            ObjectVersionUE5::UNKNOWN,
            false,
            name_map,
        )
    }

    fn package_data_entry_bytes(has_hash: bool, include_recook: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.write_i32::<LE>(0).unwrap();
        bytes.write_i32::<LE>(0).unwrap();
        bytes.write_i64::<LE>(1234).unwrap();
        for word in [0x11223344, 0x55667788, 0x99AABBCC, 0xDDEEFF00] {
            bytes.write_u32::<LE>(word).unwrap();
        }
        bytes.write_u32::<LE>(u32::from(has_hash)).unwrap();
        if has_hash {
            bytes.extend_from_slice(&[0xAB; 16]);
        }
        if include_recook {
            bytes.write_u32::<LE>(0).unwrap();
        }
        bytes
    }

    fn dev_registry_reader_with_version(version: i32) -> Reader {
        let mut bytes = Vec::new();
        for word in ASSET_REGISTRY_GUID {
            bytes.write_u32::<LE>(word).unwrap();
        }
        bytes.write_i32::<LE>(version).unwrap();

        RawReader::new(
            Chain::new(Cursor::new(bytes), None),
            ObjectVersion::VER_UE4_ASSETREGISTRY_DEPENDENCYFLAGS,
            ObjectVersionUE5::UNKNOWN,
            false,
            NameMap::new(),
        )
    }

    #[test]
    fn dev_package_data_consumes_four_byte_md5_valid_flag_and_recook_when_hash_absent() {
        let bytes = package_data_entry_bytes(false, true);
        let mut reader = package_data_reader(bytes);

        let (pkg_name, guid) = read_dev_package_data_entry(&mut reader, 8).unwrap();

        assert_eq!(pkg_name, "/Game/TestPackage");
        assert_eq!(guid, "{11223344-55667788-99AABBCC-DDEEFF00}");
        assert_eq!(reader.seek(SeekFrom::Current(0)).unwrap(), 40);
    }

    #[test]
    fn dev_package_data_consumes_four_byte_md5_valid_flag_and_recook_when_hash_present() {
        let bytes = package_data_entry_bytes(true, true);
        let mut reader = package_data_reader(bytes);

        let (pkg_name, guid) = read_dev_package_data_entry(&mut reader, 8).unwrap();

        assert_eq!(pkg_name, "/Game/TestPackage");
        assert_eq!(guid, "{11223344-55667788-99AABBCC-DDEEFF00}");
        assert_eq!(reader.seek(SeekFrom::Current(0)).unwrap(), 56);
    }

    #[test]
    fn dev_package_data_omits_recook_before_letsgos_added_recook_version() {
        let bytes = package_data_entry_bytes(false, false);
        let mut reader = package_data_reader(bytes);

        read_dev_package_data_entry(&mut reader, 7).unwrap();

        assert_eq!(reader.seek(SeekFrom::Current(0)).unwrap(), 36);
    }

    #[test]
    fn dev_package_data_keeps_alignment_across_consecutive_entries() {
        let mut bytes = package_data_entry_bytes(true, true);
        bytes.extend(package_data_entry_bytes(false, true));
        let mut reader = package_data_reader(bytes);

        read_dev_package_data_entry(&mut reader, 8).unwrap();
        let (pkg_name, guid) = read_dev_package_data_entry(&mut reader, 8).unwrap();

        assert_eq!(pkg_name, "/Game/TestPackage");
        assert_eq!(guid, "{11223344-55667788-99AABBCC-DDEEFF00}");
        assert_eq!(reader.seek(SeekFrom::Current(0)).unwrap(), 96);
    }

    #[test]
    fn dev_registry_rejects_versions_newer_than_supported_letsgos_format() {
        let mut reader = dev_registry_reader_with_version(9);

        let result = parse_dev_registry(&mut reader, None, false, &FilterConfig::default());
        let err = match result {
            Ok(_) => panic!("version 9 should be rejected"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("max supported: 8"));
    }
}
