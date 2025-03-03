use crate::error::VcxResult;
use crate::messages::a2a::{A2AMessage, MessageId};
use crate::messages::ack::PleaseAck;
use crate::messages::attachment::{AttachmentId, Attachments};
use crate::messages::thread::Thread;
use crate::messages::timing::Timing;
use crate::timing_optional;

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, Default)]
pub struct Credential {
    #[serde(rename = "@id")]
    pub id: MessageId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(rename = "credentials~attach")]
    pub credentials_attach: Attachments,
    #[serde(rename = "~thread")]
    pub thread: Thread,
    #[serde(rename = "~please_ack")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub please_ack: Option<PleaseAck>,
    #[serde(rename = "~timing")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing: Option<Timing>,
}

please_ack!(Credential);
threadlike!(Credential);
a2a_message!(Credential);
timing_optional!(Credential);

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, Default)]
pub struct CredentialData {
    pub schema_id: String,
    pub cred_def_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rev_reg_id: Option<String>,
    pub values: serde_json::Value,
    pub signature: serde_json::Value,
    pub signature_correctness_proof: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rev_reg: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub witness: Option<serde_json::Value>,
}

impl Credential {
    pub fn create() -> Self {
        Credential::default()
    }

    pub fn set_comment(mut self, comment: String) -> Self {
        self.comment = Some(comment);
        self
    }

    pub fn set_credential(mut self, credential: String) -> VcxResult<Credential> {
        self.credentials_attach
            .add_base64_encoded_json_attachment(AttachmentId::Credential, ::serde_json::Value::String(credential))?;
        Ok(self)
    }
}

#[cfg(feature = "test_utils")]
pub mod test_utils {
    use crate::messages::a2a::MessageId;
    use crate::messages::attachment::{AttachmentId, Attachments};
    use crate::messages::issuance::credential::Credential;
    use crate::messages::issuance::credential_offer::test_utils::thread;
    use crate::utils::constants;

    pub fn _attachment() -> ::serde_json::Value {
        json!({
            "schema_id":"NcYxiDXkpYi6ov5FcYDi1e:2:gvt:1.0",
            "cred_def_id":"NcYxiDXkpYi6ov5FcYDi1e:3:CL:NcYxiDXkpYi6ov5FcYDi1e:2:gvt:1.0:TAG1",
            "rev_reg_id": constants::REV_REG_ID.to_string(),
            "values":{"name":{"raw":"Name","encoded":"1139481716457488690172217916278103335"}}
        })
    }

    pub fn _comment() -> String {
        String::from("comment")
    }

    pub fn _credential() -> Credential {
        let mut attachment = Attachments::new();
        attachment
            .add_base64_encoded_json_attachment(AttachmentId::Credential, _attachment())
            .unwrap();

        Credential {
            id: MessageId::id(),
            comment: Some(_comment()),
            thread: thread(),
            credentials_attach: attachment,
            please_ack: None,
            timing: None,
        }
    }
}

#[cfg(test)]
#[cfg(feature = "general_test")]
pub mod unit_tests {
    use crate::messages::issuance::credential::test_utils::*;
    use crate::messages::issuance::credential_offer::test_utils::thread_id;

    use super::*;

    #[test]
    fn test_credential_build_works() {
        let credential: Credential = Credential::create()
            .set_comment(_comment())
            .set_thread_id(&thread_id())
            .set_credential(_attachment().to_string())
            .unwrap();

        assert_eq!(_credential(), credential);
    }
}
