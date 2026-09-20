use anyhow::{Context, Result};
use sequoia_openpgp::{
    parse::{stream::*, Parse},
    policy::StandardPolicy,
    Cert,
};

const PUBLIC_KEY: &[u8] = include_bytes!("../pub.pgp.asc");

pub fn verify_signature(data: &str, signature: &str) -> Result<()> {
    let policy = StandardPolicy::new();
    let cert = Cert::from_bytes(PUBLIC_KEY).context("Failed to parse embedded GPG public key")?;
    let helper = Helper { cert: &cert };
    let mut verifier = DetachedVerifierBuilder::from_bytes(signature.as_bytes())?
        .with_policy(&policy, None, helper)
        .context("GPG signature structure invalid")?;
    verifier.verify_bytes(data.as_bytes())?;
    Ok(())
}

struct Helper<'a> {
    cert: &'a Cert,
}

impl VerificationHelper for Helper<'_> {
    fn get_certs(
        &mut self,
        _ids: &[sequoia_openpgp::KeyHandle],
    ) -> sequoia_openpgp::Result<Vec<Cert>> {
        Ok(vec![self.cert.clone()])
    }
    fn check(&mut self, structure: MessageStructure) -> sequoia_openpgp::Result<()> {
        for layer in structure {
            if let MessageLayer::SignatureGroup { results } = layer {
                if results.into_iter().any(|r| r.is_ok()) {
                    return Ok(());
                }
            }
        }
        Err(anyhow::anyhow!(
            "No valid GPG signature found from trusted key"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_real_signature() {
        let hash_sha256 = "859de61af40ea0b3f836af6d8eb801d5ed25e7582f2f7f47e0ffe3d2e149bdc7  linux-amd64.zip\n1f0d5610a29bd70198e7fe8c31d82c9acb39d23b7e659cdab9646dc117466674  linux-arm64.zip\n817cacddcc91691b7691e03060c2dfe5a04325df8805434c88032b70b06519d9  windows-amd64.zip\n";
        let sign_gpg = "-----BEGIN PGP SIGNATURE-----\n\niQIzBAABCgAdFiEE09RBOxFH8cabfO/ja9UaMJ3zQ88FAmqlrzoACgkQa9UaMJ3z\nQ88Zwg//QYcKFcWIMnWFozaGozBD9Z+ZxjT9DlgBuaMY+XSejFCJUedjlYvGu+BY\n32v7pvF817hRUeaSk+Rai0fAC6uROycFWHJLTMW9kZs+JkLnJSjxBOkVUelH9Xzg\nGCOVHja40xrgBFFqz9/mqK8g8uHWhEdWi3X7ew1yAAyJlDafHlILyIpos0I7C+81\noocEYz5Pl79WQXKSXFrh7Oy4IhzRMWth4GFkA4UalJGS1ZBhkqL5iNGrr1zBhVXI\nPl8oIHuZtCXN9UELiTrZBzpwHEm6lok1iXsYfPzW/qrR8pLE7SOeaV5MjZ+Zvk83\n+dWU4DIRAaJoGI/eeJMajU3uZ6CeNlCo6ML7x2O9xNROghjJR7o/35L/Ud8+La3U\nuqoZ8Xjgxb6mHP0XDD7RarQO+ONgG6+aSKdGMygh5kQpDDDApDlRPZtft9pjoPpk\n4bqu2j6C1dL/5A0U11lqSliVvbSpewzTyEkZaQPMs8rY1yF7OHTz8j2NTVk/Ijm+\na1Wz8XpKJLBD836M83Pg3g3TOa6g2d4aYFmW7UZyl87Y0Uoc41mHvL5amRhAKZD8\njduFpml0Me3kYEbWOl3OM6yatzy+H7dib7/apNFScfl22ULGsxdf83H40gvnGMJG\n8o/SVYZIoGMor0gjjbi/TxSlMpY3KmKwaitCTnV/Pq+H7B96YJs=\n=Hoq5\n-----END PGP SIGNATURE-----\n";

        assert!(verify_signature(hash_sha256, sign_gpg).is_ok());

        // Tampered data should fail
        let tampered_data =
            "0000000000000000000000000000000000000000000000000000000000000000  linux-amd64.zip\n";
        assert!(verify_signature(tampered_data, sign_gpg).is_err());
    }

    #[test]
    fn test_verify_corrupted_signature() {
        let hash_sha256 =
            "859de61af40ea0b3f836af6d8eb801d5ed25e7582f2f7f47e0ffe3d2e149bdc7  linux-amd64.zip\n";
        let invalid_sig =
            "-----BEGIN PGP SIGNATURE-----\ncorrupted data\n-----END PGP SIGNATURE-----";
        assert!(verify_signature(hash_sha256, invalid_sig).is_err());

        assert!(verify_signature(hash_sha256, "not even pgp armor").is_err());
        assert!(verify_signature(hash_sha256, "").is_err());
        assert!(verify_signature("", invalid_sig).is_err());
    }
}
