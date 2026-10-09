//! What a downloaded update must prove before the updater installs it.
//!
//! These are the platform-independent decisions; `installer.rs` extracts the
//! inputs on Windows (the signer chain WinVerifyTrust built, and the
//! VERSIONINFO strings of the verified file) and calls in here. Everything
//! is exact matching on parsed values: no substring, prefix or
//! case-insensitive comparisons.

/// Subject CN and O of the certificate that makes the primary signature on
/// every release binary (Trusted Signing profile faraCodeSigningBusiness /
/// Faratech, see DUAL-SIGNING.md). The leaf rotates every few days, so the
/// subject is pinned, never a thumbprint.
pub const UPDATE_PUBLISHER: &str = "Fara Technologies LLC";

/// Microsoft's Trusted Signing hierarchy above the issuing CA:
/// leaf -> "Microsoft ID Verified CS EOC CA nn" -> PCA 2021 -> Root 2020.
/// The EOC issuing CA rotates (01, 02, 03, ...), so only its organization is
/// checked; the PCA (valid to 2036) and root (to 2045) are pinned by name.
pub const MICROSOFT_ORGANIZATION: &str = "Microsoft Corporation";
pub const TRUSTED_SIGNING_PCA: &str = "Microsoft ID Verified Code Signing PCA 2021";
pub const TRUSTED_SIGNING_ROOT: &str =
    "Microsoft Identity Verification Root Certificate Authority 2020";

/// EKU Trusted Signing puts on every public-trust signing certificate.
pub const TRUSTED_SIGNING_EKU: &str = "1.3.6.1.4.1.311.97.1.0";
pub const CODE_SIGNING_EKU: &str = "1.3.6.1.5.5.7.3.3";

pub const OID_COMMON_NAME: &str = "2.5.4.3";
pub const OID_ORGANIZATION: &str = "2.5.4.10";

/// VERSIONINFO strings media/htop.rc stamps into every build.
pub const UPDATE_PRODUCT_NAME: &str = "htop-win";
pub const UPDATE_ORIGINAL_FILENAME: &str = "htop-win.exe";
pub const UPDATE_INTERNAL_NAME: &str = "htop-win";

/// A certificate subject as parsed RDN attributes, `(OID, value)` in order.
pub type Subject = Vec<(String, String)>;

/// The certificate chain WinVerifyTrust built for the primary signature.
#[derive(Debug, Clone, Default)]
pub struct SignerChain {
    /// Subjects from the signing certificate (index 0) up to the root.
    pub subjects: Vec<Subject>,
    /// Extended key usages of the signing certificate.
    pub leaf_ekus: Vec<String>,
}

/// The value of `oid` when the subject has exactly one such attribute.
fn single_attribute<'a>(subject: &'a [(String, String)], oid: &str) -> Option<&'a str> {
    let mut values = subject
        .iter()
        .filter(|(attribute, _)| attribute == oid)
        .map(|(_, value)| value.as_str());
    let first = values.next()?;
    values.next().is_none().then_some(first)
}

fn subject_is(subject: &[(String, String)], common_name: &str, organization: &str) -> bool {
    single_attribute(subject, OID_COMMON_NAME) == Some(common_name)
        && single_attribute(subject, OID_ORGANIZATION) == Some(organization)
}

fn describe(subject: &[(String, String)]) -> String {
    format!(
        "CN={:?}, O={:?}",
        single_attribute(subject, OID_COMMON_NAME).unwrap_or("?"),
        single_attribute(subject, OID_ORGANIZATION).unwrap_or("?")
    )
}

/// Accept only a signature by [`UPDATE_PUBLISHER`] issued through Microsoft's
/// Trusted Signing hierarchy.
pub fn check_signer_chain(chain: &SignerChain) -> Result<(), String> {
    let [leaf, issuing_ca, pca, root, ..] = chain.subjects.as_slice() else {
        return Err(format!(
            "signer chain has {} certificates, expected the 4-level Trusted Signing chain",
            chain.subjects.len()
        ));
    };
    if !subject_is(leaf, UPDATE_PUBLISHER, UPDATE_PUBLISHER) {
        return Err(format!(
            "unexpected publisher: {}, expected CN and O {UPDATE_PUBLISHER:?}",
            describe(leaf)
        ));
    }
    if single_attribute(issuing_ca, OID_ORGANIZATION) != Some(MICROSOFT_ORGANIZATION)
        || !subject_is(pca, TRUSTED_SIGNING_PCA, MICROSOFT_ORGANIZATION)
        || !subject_is(root, TRUSTED_SIGNING_ROOT, MICROSOFT_ORGANIZATION)
    {
        return Err(format!(
            "signature is not from the Trusted Signing hierarchy: {} / {} / {}",
            describe(issuing_ca),
            describe(pca),
            describe(root)
        ));
    }
    for eku in [CODE_SIGNING_EKU, TRUSTED_SIGNING_EKU] {
        if !chain.leaf_ekus.iter().any(|usage| usage == eku) {
            return Err(format!("signing certificate lacks EKU {eku}"));
        }
    }
    Ok(())
}

/// VERSIONINFO string fields read from the verified update file.
#[derive(Debug, Clone, Default)]
pub struct VersionStrings {
    pub product_name: Option<String>,
    pub original_filename: Option<String>,
    pub internal_name: Option<String>,
    pub file_version: Option<String>,
}

/// Which FileVersion an update may carry.
#[derive(Clone, Copy, Debug)]
pub enum VersionRule<'a> {
    /// Exactly this release version, strictly newer than the running build.
    Release(&'a str),
    /// `htop --update --force`: exactly this release version, which may
    /// equal the running build (a reinstall) but never be older.
    Reinstall(&'a str),
    /// Any version strictly newer than the running build.
    NewerThanRunning,
}

/// Accept only an htop-win binary whose FileVersion satisfies `rule`, so a
/// validly signed but different Fara Technologies program, or an older
/// htop-win (rollback), is rejected. `is_newer(a, b)` is SemVer `a > b`.
pub fn check_update_identity(
    strings: &VersionStrings,
    rule: VersionRule<'_>,
    running: &str,
    is_newer: impl Fn(&str, &str) -> bool,
) -> Result<(), String> {
    for (field, value, expected) in [
        ("ProductName", &strings.product_name, UPDATE_PRODUCT_NAME),
        (
            "OriginalFilename",
            &strings.original_filename,
            UPDATE_ORIGINAL_FILENAME,
        ),
        ("InternalName", &strings.internal_name, UPDATE_INTERNAL_NAME),
    ] {
        match value.as_deref() {
            Some(value) if value == expected => {}
            Some(value) => return Err(format!("{field} is {value:?}, expected {expected:?}")),
            None => return Err(format!("version resource has no {field}")),
        }
    }
    let version = strings
        .file_version
        .as_deref()
        .ok_or("version resource has no FileVersion")?;
    let expected = match rule {
        VersionRule::Release(expected) | VersionRule::Reinstall(expected) => Some(expected),
        VersionRule::NewerThanRunning => None,
    };
    if let Some(expected) = expected
        && version != expected
    {
        return Err(format!(
            "FileVersion {version:?} does not match the release version {expected:?}"
        ));
    }
    match rule {
        VersionRule::Reinstall(_) if is_newer(running, version) => Err(format!(
            "FileVersion {version} is older than the running {running}"
        )),
        VersionRule::Release(_) | VersionRule::NewerThanRunning if !is_newer(version, running) => {
            Err(format!(
                "FileVersion {version} is not newer than the running {running}"
            ))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject(attributes: &[(&str, &str)]) -> Subject {
        attributes
            .iter()
            .map(|(oid, value)| (oid.to_string(), value.to_string()))
            .collect()
    }

    /// The chain of the v0.2.13 release binaries.
    fn release_chain() -> SignerChain {
        SignerChain {
            subjects: vec![
                subject(&[
                    ("2.5.4.6", "US"),
                    ("2.5.4.8", "New York"),
                    ("2.5.4.7", "White Plains"),
                    (OID_ORGANIZATION, "Fara Technologies LLC"),
                    (OID_COMMON_NAME, "Fara Technologies LLC"),
                ]),
                subject(&[
                    ("2.5.4.6", "US"),
                    (OID_ORGANIZATION, "Microsoft Corporation"),
                    (OID_COMMON_NAME, "Microsoft ID Verified CS EOC CA 03"),
                ]),
                subject(&[
                    ("2.5.4.6", "US"),
                    (OID_ORGANIZATION, "Microsoft Corporation"),
                    (OID_COMMON_NAME, TRUSTED_SIGNING_PCA),
                ]),
                subject(&[
                    ("2.5.4.6", "US"),
                    (OID_ORGANIZATION, "Microsoft Corporation"),
                    (OID_COMMON_NAME, TRUSTED_SIGNING_ROOT),
                ]),
            ],
            leaf_ekus: vec![
                TRUSTED_SIGNING_EKU.to_string(),
                CODE_SIGNING_EKU.to_string(),
                "1.3.6.1.4.1.311.97.107654787.18607814.94366046.634093174".to_string(),
            ],
        }
    }

    fn with_leaf(attributes: &[(&str, &str)]) -> SignerChain {
        let mut chain = release_chain();
        chain.subjects[0] = subject(attributes);
        chain
    }

    #[test]
    fn release_signer_chain_is_accepted() {
        assert_eq!(check_signer_chain(&release_chain()), Ok(()));
    }

    #[test]
    fn leaf_subject_must_match_exactly_and_once() {
        let fara = "Fara Technologies LLC";
        for leaf in [
            vec![
                (OID_ORGANIZATION, "Mike Fara"),
                (OID_COMMON_NAME, "Mike Fara"),
            ],
            vec![
                (OID_ORGANIZATION, fara),
                (OID_COMMON_NAME, "Fara Technologies LLC Evil"),
            ],
            vec![
                (OID_ORGANIZATION, fara),
                (OID_COMMON_NAME, "Evil Fara Technologies LLC"),
            ],
            vec![
                (OID_ORGANIZATION, fara),
                (OID_COMMON_NAME, "fara technologies llc"),
            ],
            vec![
                (OID_ORGANIZATION, fara),
                (OID_COMMON_NAME, "Fara Technologies LLC "),
            ],
            vec![(OID_ORGANIZATION, "Evil LLC"), (OID_COMMON_NAME, fara)],
            vec![(OID_COMMON_NAME, fara)],
            vec![(OID_ORGANIZATION, fara)],
            // Duplicate attributes: the matching one must be the only one.
            vec![
                (OID_ORGANIZATION, fara),
                (OID_COMMON_NAME, fara),
                (OID_COMMON_NAME, "Evil"),
            ],
            vec![
                (OID_ORGANIZATION, "Evil"),
                (OID_ORGANIZATION, fara),
                (OID_COMMON_NAME, fara),
            ],
            // The pinned names in other attributes (OU, L) are not CN/O.
            vec![("2.5.4.11", fara), ("2.5.4.7", fara)],
        ] {
            let error = check_signer_chain(&with_leaf(&leaf)).expect_err("leaf must be rejected");
            assert!(
                error.starts_with("unexpected publisher"),
                "{leaf:?}: {error}"
            );
        }
    }

    #[test]
    fn chain_must_come_from_the_trusted_signing_hierarchy() {
        let mut other_root = release_chain();
        other_root.subjects[3] = subject(&[
            (OID_ORGANIZATION, "DigiCert Inc"),
            (OID_COMMON_NAME, "DigiCert Trusted Root G4"),
        ]);
        let mut other_pca = release_chain();
        other_pca.subjects[2] = subject(&[
            (OID_ORGANIZATION, "Microsoft Corporation"),
            (OID_COMMON_NAME, "Microsoft Code Signing PCA 2011"),
        ]);
        let mut other_issuer = release_chain();
        other_issuer.subjects[1] = subject(&[
            (OID_ORGANIZATION, "Evil CA"),
            (OID_COMMON_NAME, "Microsoft ID Verified CS EOC CA 03"),
        ]);
        let mut lookalike_root = release_chain();
        lookalike_root.subjects[3] = subject(&[
            (OID_ORGANIZATION, "Microsoft Corporation"),
            (
                OID_COMMON_NAME,
                "Microsoft Identity Verification Root Certificate Authority 2020 Evil",
            ),
        ]);
        let mut short = release_chain();
        short.subjects.truncate(3);

        for chain in [other_root, other_pca, other_issuer, lookalike_root, short] {
            let error = check_signer_chain(&chain).expect_err("chain must be rejected");
            assert!(!error.starts_with("unexpected publisher"), "{error}");
        }
    }

    #[test]
    fn signing_certificate_needs_the_trusted_signing_and_code_signing_ekus() {
        for missing in [TRUSTED_SIGNING_EKU, CODE_SIGNING_EKU] {
            let mut chain = release_chain();
            chain.leaf_ekus.retain(|usage| usage != missing);
            let error = check_signer_chain(&chain).expect_err("EKU must be required");
            assert!(error.contains(missing), "{error}");
        }
    }

    /// Plain numeric x.y.z comparison, enough for these fixtures.
    fn newer(a: &str, b: &str) -> bool {
        let parse = |v: &str| -> Vec<u64> { v.split('.').map(|n| n.parse().unwrap()).collect() };
        parse(a) > parse(b)
    }

    fn strings(version: &str) -> VersionStrings {
        VersionStrings {
            product_name: Some("htop-win".into()),
            original_filename: Some("htop-win.exe".into()),
            internal_name: Some("htop-win".into()),
            file_version: Some(version.into()),
        }
    }

    #[test]
    fn release_update_must_be_this_release_and_newer() {
        let release = VersionRule::Release("0.2.14");
        assert_eq!(
            check_update_identity(&strings("0.2.14"), release, "0.2.13", newer),
            Ok(())
        );
        // A signed older htop-win (rollback), a different version than the
        // release names, and the running version itself are all refused.
        for (version, running) in [
            ("0.2.12", "0.2.13"),
            ("0.2.15", "0.2.13"),
            ("0.2.14", "0.2.14"),
        ] {
            assert!(
                check_update_identity(&strings(version), release, running, newer).is_err(),
                "{version} over {running}"
            );
        }
    }

    #[test]
    fn forced_reinstall_allows_the_same_version_but_never_an_older_one() {
        assert_eq!(
            check_update_identity(
                &strings("0.2.13"),
                VersionRule::Reinstall("0.2.13"),
                "0.2.13",
                newer
            ),
            Ok(())
        );
        assert!(
            check_update_identity(
                &strings("0.2.12"),
                VersionRule::Reinstall("0.2.12"),
                "0.2.13",
                newer
            )
            .is_err()
        );
        assert!(
            check_update_identity(
                &strings("0.2.12"),
                VersionRule::Reinstall("0.2.13"),
                "0.2.13",
                newer
            )
            .is_err()
        );
    }

    #[test]
    fn local_install_must_be_newer_than_running() {
        let rule = VersionRule::NewerThanRunning;
        assert_eq!(
            check_update_identity(&strings("0.3.0"), rule, "0.2.13", newer),
            Ok(())
        );
        assert!(check_update_identity(&strings("0.2.13"), rule, "0.2.13", newer).is_err());
        assert!(check_update_identity(&strings("0.1.0"), rule, "0.2.13", newer).is_err());
    }

    #[test]
    fn other_programs_and_missing_version_info_are_refused() {
        let rule = VersionRule::Release("0.2.14");
        let mut other_product = strings("0.2.14");
        other_product.product_name = Some("fwdslash".into());
        let mut renamed = strings("0.2.14");
        renamed.original_filename = Some("fsw-broker.exe".into());
        let mut other_internal = strings("0.2.14");
        other_internal.internal_name = Some("htop-win-helper".into());
        let mut lookalike = strings("0.2.14");
        lookalike.product_name = Some("htop-win ".into());
        let mut no_version = strings("0.2.14");
        no_version.file_version = None;
        let mut no_product = strings("0.2.14");
        no_product.product_name = None;

        for (case, info) in [
            ("other product", other_product),
            ("other file name", renamed),
            ("other internal name", other_internal),
            ("lookalike product", lookalike),
            ("no FileVersion", no_version),
            ("no ProductName", no_product),
            ("no VERSIONINFO", VersionStrings::default()),
        ] {
            assert!(
                check_update_identity(&info, rule, "0.2.13", newer).is_err(),
                "{case} accepted"
            );
        }
    }
}
