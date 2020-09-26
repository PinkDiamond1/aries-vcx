use openssl;
use openssl::bn::{BigNum, BigNumRef};
use serde_json;
use serde_json::Value;

use api::{ProofStateType, VcxStateType};
use error::prelude::*;
use messages;
use messages::{GeneralMessage, RemoteMessageType};
use messages::get_message::get_ref_msg;
use messages::payload::{PayloadKinds, Payloads};
use messages::proofs::proof_message::{CredInfo, ProofMessage};
use messages::proofs::proof_message::get_credential_info;
use messages::proofs::proof_request::{ProofRequestMessage, ProofRequestVersion};
use messages::thread::Thread;
use object_cache::ObjectCache;
use settings;
use settings::get_config_value;
use utils::agent_info::{get_agent_attr, get_agent_info, MyAgentInfo};
use utils::constants::*;
use utils::error;
use utils::libindy::anoncreds;
use utils::openssl::encode;
use utils::qualifier;
use v3::handlers::proof_presentation::verifier::verifier::Verifier;

lazy_static! {
    static ref PROOF_MAP: ObjectCache<Verifier> = ObjectCache::<Verifier>::new("proofs-cache");
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "version", content = "data")]
enum Proofs {
    #[serde(rename = "3.0")]
    Pending(Proof),
    #[serde(rename = "1.0")]
    V1(Proof),
    #[serde(rename = "2.0")]
    V3(Verifier),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct RevocationInterval {
    from: Option<u64>,
    to: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Proof {
    source_id: String,
    requested_attrs: String,
    requested_predicates: String,
    msg_uid: String,
    ref_msg_id: String,
    state: VcxStateType,
    proof_state: ProofStateType,
    name: String,
    version: String,
    nonce: String,
    proof: Option<ProofMessage>,
    // Refactoring this name to 'proof_message' causes some tests to fail.
    proof_request: Option<ProofRequestMessage>,
    #[serde(rename = "prover_did")]
    my_did: Option<String>,
    #[serde(rename = "prover_vk")]
    my_vk: Option<String>,
    #[serde(rename = "remote_did")]
    their_did: Option<String>,
    #[serde(rename = "remote_vk")]
    their_vk: Option<String>,
    agent_did: Option<String>,
    agent_vk: Option<String>,
    revocation_interval: RevocationInterval,
    thread: Option<Thread>,
}

impl Proof {
    pub fn create(source_id: String,
                  requested_attrs: String,
                  requested_predicates: String,
                  revocation_details: String,
                  name: String) -> VcxResult<Proof> {
        trace!("create >>> source_id: {}, requested_attrs: {}, requested_predicates: {}, name: {}", source_id, requested_attrs, requested_predicates, name);

        // TODO: Get this to actually validate as json, not just check length.
        if requested_attrs.len() <= 0 { return Err(VcxError::from(VcxErrorKind::InvalidJson)); }

        let revocation_details: RevocationInterval = serde_json::from_str(&revocation_details)
            .or(Err(VcxError::from(VcxErrorKind::InvalidJson)))?;

        debug!("creating proof with source_id: {}, name: {}, requested_attrs: {}, requested_predicates: {}", source_id, name, requested_attrs, requested_predicates);

        let mut new_proof = Proof {
            source_id,
            requested_attrs,
            requested_predicates,
            name,
            msg_uid: String::new(),
            ref_msg_id: String::new(),
            state: VcxStateType::VcxStateNone,
            proof_state: ProofStateType::ProofUndefined,
            version: String::from("1.0"),
            nonce: generate_nonce()?,
            proof: None,
            proof_request: None,
            revocation_interval: revocation_details,
            my_did: None,
            my_vk: None,
            their_did: None,
            their_vk: None,
            agent_did: None,
            agent_vk: None,
            thread: Some(Thread::new()),
        };

        new_proof.state = VcxStateType::VcxStateInitialized;

        Ok(new_proof)
    }

    pub fn validate_proof_revealed_attributes(proof_json: &str) -> VcxResult<()> {
        if settings::indy_mocks_enabled() { return Ok(()); }

        let proof: Value = serde_json::from_str(proof_json)
            .map_err(|err| VcxError::from_msg(VcxErrorKind::InvalidJson, format!("Cannot deserialize libndy proof: {}", err)))?;

        let revealed_attrs = match proof["requested_proof"]["revealed_attrs"].as_object() {
            Some(revealed_attrs) => revealed_attrs,
            None => return Ok(())
        };

        for (attr1_referent, info) in revealed_attrs.iter() {
            let raw = info["raw"].as_str().ok_or(VcxError::from_msg(VcxErrorKind::InvalidProof, format!("Cannot get raw value for \"{}\" attribute", attr1_referent)))?;
            let encoded_ = info["encoded"].as_str().ok_or(VcxError::from_msg(VcxErrorKind::InvalidProof, format!("Cannot get encoded value for \"{}\" attribute", attr1_referent)))?;

            let expected_encoded = encode(&raw)?;

            if expected_encoded != encoded_.to_string() {
                return Err(VcxError::from_msg(VcxErrorKind::InvalidProof, format!("Encoded values are different. Expected: {}. From Proof: {}", expected_encoded, encoded_)));
            }
        }

        Ok(())
    }

    pub fn build_credential_defs_json(credential_data: &Vec<CredInfo>) -> VcxResult<String> {
        debug!("building credential_def_json for proof validation");
        let mut credential_json = json!({});

        for ref cred_info in credential_data.iter() {
            if credential_json.get(&cred_info.cred_def_id).is_none() {
                let (id, credential_def) = anoncreds::get_cred_def_json(&cred_info.cred_def_id)?;

                let credential_def = serde_json::from_str(&credential_def)
                    .map_err(|err| VcxError::from_msg(VcxErrorKind::InvalidProofCredentialData, format!("Cannot deserialize credential definition: {}", err)))?;

                credential_json[id] = credential_def;
            }
        }

        Ok(credential_json.to_string())
    }

    pub fn build_schemas_json(credential_data: &Vec<CredInfo>) -> VcxResult<String> {
        debug!("building schemas json for proof validation");

        let mut schemas_json = json!({});

        for ref cred_info in credential_data.iter() {
            if schemas_json.get(&cred_info.schema_id).is_none() {
                let (id, schema_json) = anoncreds::get_schema_json(&cred_info.schema_id)
                    .map_err(|err| err.map(VcxErrorKind::InvalidSchema, "Cannot get schema"))?;

                let schema_val = serde_json::from_str(&schema_json)
                    .map_err(|err| VcxError::from_msg(VcxErrorKind::InvalidSchema, format!("Cannot deserialize schema: {}", err)))?;

                schemas_json[id] = schema_val;
            }
        }

        Ok(schemas_json.to_string())
    }

    pub fn build_rev_reg_defs_json(credential_data: &Vec<CredInfo>) -> VcxResult<String> {
        debug!("building rev_reg_def_json for proof validation");

        let mut rev_reg_defs_json = json!({});

        for ref cred_info in credential_data.iter() {
            let rev_reg_id = cred_info
                .rev_reg_id
                .as_ref()
                .ok_or(VcxError::from(VcxErrorKind::InvalidRevocationDetails))?;

            if rev_reg_defs_json.get(rev_reg_id).is_none() {
                let (id, json) = anoncreds::get_rev_reg_def_json(rev_reg_id)
                    .or(Err(VcxError::from(VcxErrorKind::InvalidRevocationDetails)))?;

                let rev_reg_def_json = serde_json::from_str(&json)
                    .or(Err(VcxError::from(VcxErrorKind::InvalidSchema)))?;

                rev_reg_defs_json[id] = rev_reg_def_json;
            }
        }

        Ok(rev_reg_defs_json.to_string())
    }

    pub fn build_rev_reg_json(credential_data: &Vec<CredInfo>) -> VcxResult<String> {
        debug!("building rev_reg_json for proof validation");

        let mut rev_regs_json = json!({});

        for ref cred_info in credential_data.iter() {
            let rev_reg_id = cred_info
                .rev_reg_id
                .as_ref()
                .ok_or(VcxError::from(VcxErrorKind::InvalidRevocationDetails))?;

            let timestamp = cred_info
                .timestamp
                .as_ref()
                .ok_or(VcxError::from(VcxErrorKind::InvalidRevocationTimestamp))?;

            if rev_regs_json.get(rev_reg_id).is_none() {
                let (id, json, timestamp) = anoncreds::get_rev_reg(rev_reg_id, timestamp.to_owned())
                    .or(Err(VcxError::from(VcxErrorKind::InvalidRevocationDetails)))?;

                let rev_reg_json: Value = serde_json::from_str(&json)
                    .or(Err(VcxError::from(VcxErrorKind::InvalidJson)))?;

                let rev_reg_json = json!({timestamp.to_string(): rev_reg_json});
                rev_regs_json[id] = rev_reg_json;
            }
        }

        Ok(rev_regs_json.to_string())
    }

    fn build_proof_json(&self) -> VcxResult<String> {
        debug!("{} building proof json for proof validation", self.source_id);
        match self.proof {
            Some(ref x) => Ok(x.libindy_proof.clone()),
            None => Err(VcxError::from(VcxErrorKind::InvalidProof)),
        }
    }

    fn build_proof_req_json(&self) -> VcxResult<String> {
        debug!("{} building proof request json for proof validation", self.source_id);
        if let Some(ref x) = self.proof_request {
            return Ok(x.get_proof_request_data());
        }
        Err(VcxError::from(VcxErrorKind::InvalidProof))
    }

    fn proof_validation(&mut self) -> VcxResult<u32> {
        let proof_json = self.build_proof_json()?;
        let proof_req_json = self.build_proof_req_json()?;

        let valid = Proof::validate_indy_proof(&proof_json, &proof_req_json).map_err(|err| {
            error!("Error: {}, Proof {} wasn't valid", err, self.source_id);
            self.proof_state = ProofStateType::ProofInvalid;
            err.map(VcxErrorKind::InvalidProof, error::INVALID_PROOF.message)
        })?;

        if !valid {
            warn!("indy returned false when validating proof {}", self.source_id);
            self.proof_state = ProofStateType::ProofInvalid;
            return Ok(error::SUCCESS.code_num);
        }

        debug!("Indy validated proof: {}", self.source_id);
        self.proof_state = ProofStateType::ProofValidated;
        Ok(error::SUCCESS.code_num)
    }

    pub fn validate_indy_proof(proof_json: &str, proof_req_json: &str) -> VcxResult<bool> {
        if settings::indy_mocks_enabled() {
            let mock_result: bool = get_config_value(settings::MOCK_INDY_PROOF_VALIDATION).unwrap_or("true".into()).parse().unwrap();
            return Ok(mock_result);
        }

        Proof::validate_proof_revealed_attributes(&proof_json)?;

        let credential_data = get_credential_info(&proof_json)?;

        let credential_defs_json = Proof::build_credential_defs_json(&credential_data)
            .unwrap_or(json!({}).to_string());
        let schemas_json = Proof::build_schemas_json(&credential_data)
            .unwrap_or(json!({}).to_string());
        let rev_reg_defs_json = Proof::build_rev_reg_defs_json(&credential_data)
            .unwrap_or(json!({}).to_string());
        let rev_regs_json = Proof::build_rev_reg_json(&credential_data)
            .unwrap_or(json!({}).to_string());

        debug!("*******\n{}\n********", credential_defs_json);
        debug!("*******\n{}\n********", schemas_json);
        debug!("*******\n{}\n********", proof_json);
        debug!("*******\n{}\n********", proof_req_json);
        debug!("*******\n{}\n********", rev_reg_defs_json);
        debug!("*******\n{}\n********", rev_regs_json);
        anoncreds::libindy_verifier_verify_proof(proof_req_json,
                                                 proof_json,
                                                 &schemas_json,
                                                 &credential_defs_json,
                                                 &rev_reg_defs_json,
                                                 &rev_regs_json)
    }

    fn generate_proof_request_msg(&mut self) -> VcxResult<String> {
        let their_did = self.their_did.clone().unwrap_or_default();
        let version = if qualifier::is_fully_qualified(&their_did) {
            Some(ProofRequestVersion::V2)
        } else { None };

        let data_version = "0.1";
        let mut proof_obj = messages::proof_request();
        let proof_request = proof_obj
            .type_version(&self.version)?
            .proof_request_format_version(version)?
            .nonce(&self.nonce)?
            .proof_name(&self.name)?
            .proof_data_version(data_version)?
            .requested_attrs(&self.requested_attrs)?
            .requested_predicates(&self.requested_predicates)?
            .from_timestamp(self.revocation_interval.from)?
            .to_timestamp(self.revocation_interval.to)?
            .serialize_message()?;

        self.proof_request = Some(proof_obj);
        Ok(proof_request)
    }

    fn send_proof_request(&mut self, connection_handle: u32) -> VcxResult<u32> {
        trace!("Proof::send_proof_request >>> connection_handle: {}", connection_handle);

        if self.state != VcxStateType::VcxStateInitialized {
            warn!("proof {} has invalid state {} for sending proofRequest", self.source_id, self.state as u32);
            return Err(VcxError::from(VcxErrorKind::NotReady));
        }
        debug!("sending proof request with proof: {}, and connection {}", self.source_id, connection_handle);
        let agent_info = get_agent_info()?.pw_info(connection_handle)?;
        apply_agent_info(self, &agent_info);

        let title = format!("{} wants you to share: {}",
                            settings::get_config_value(settings::CONFIG_INSTITUTION_NAME)?,
                            self.name);

        let proof_request = self.generate_proof_request_msg()?;

        let response = messages::send_message()
            .to(&agent_info.my_pw_did()?)?
            .to_vk(&agent_info.my_pw_vk()?)?
            .msg_type(&RemoteMessageType::ProofReq)?
            .agent_did(&agent_info.pw_agent_did()?)?
            .agent_vk(&agent_info.pw_agent_vk()?)?
            .set_title(&title)?
            .set_detail(&title)?
            .edge_agent_payload(&agent_info.my_pw_vk()?,
                                &agent_info.their_pw_vk()?,
                                &proof_request,
                                PayloadKinds::ProofRequest,
                                self.thread.clone())
            .or(Err(VcxError::from(VcxErrorKind::InvalidConnectionHandle)))?
            .send_secure()
            .map_err(|err| err.extend("Cannot send proof request"))?;

        self.msg_uid = response.get_msg_uid()?;
        self.state = VcxStateType::VcxStateOfferSent;
        Ok(error::SUCCESS.code_num)
    }

    fn get_proof(&self) -> VcxResult<String> {
        Ok(self.proof.as_ref().ok_or(VcxError::from(VcxErrorKind::InvalidProofHandle))?.libindy_proof.clone())
    }

    fn get_proof_request_status(&mut self, message: Option<String>) -> VcxResult<u32> {
        debug!("updating state for proof {} with msg_id {:?}", self.source_id, self.msg_uid);
        if self.state == VcxStateType::VcxStateAccepted {
            return Ok(self.get_state());
        } else if message.is_none() &&
            (self.state != VcxStateType::VcxStateOfferSent || self.msg_uid.is_empty() || self.my_did.is_none()) {
            return Ok(self.get_state());
        }

        let payload = match message {
            None => {
                // Check cloud agent for pending messages
                let (_, message) = get_ref_msg(&self.msg_uid,
                                               &get_agent_attr(&self.my_did)?,
                                               &get_agent_attr(&self.my_vk)?,
                                               &get_agent_attr(&self.agent_did)?,
                                               &get_agent_attr(&self.agent_vk)?)?;

                let (payload, thread) = Payloads::decrypt(
                    &get_agent_attr(&self.my_vk)?,
                    &message,
                )?;

                if let Some(_) = thread {
                    let remote_did = &get_agent_attr(&self.their_did)?;
                    self.thread.as_mut().map(|thread| thread.increment_receiver(&remote_did));
                }

                payload
            }
            Some(ref message) => message.clone(),
        };
        debug!("proof: {}", payload);

        self.proof = match parse_proof_payload(&payload) {
            Err(_) => return Ok(self.get_state()),
            Ok(x) => {
                self.state = x.state.unwrap_or(VcxStateType::VcxStateAccepted);
                Some(x)
            }
        };

        if self.state == VcxStateType::VcxStateAccepted {
            match self.proof_validation() {
                Ok(_) => {
                    if self.proof_state != ProofStateType::ProofInvalid {
                        debug!("Proof format was validated for proof {}", self.source_id);
                        self.proof_state = ProofStateType::ProofValidated;
                    }
                }
                Err(x) => {
                    self.state = VcxStateType::VcxStateRequestReceived;
                    warn!("Proof {} had invalid format with err {}", self.source_id, x);
                    self.proof_state = ProofStateType::ProofInvalid;
                }
            };
        }

        Ok(self.get_state())
    }

    fn update_state(&mut self, message: Option<String>) -> VcxResult<u32> {
        trace!("Proof::update_state >>>");
        self.get_proof_request_status(message)
    }

    fn get_state(&self) -> u32 {
        trace!("Proof::get_state >>>");
        self.state as u32
    }

    fn get_proof_state(&self) -> u32 {
        self.proof_state as u32
    }
}

pub fn create_proof(source_id: String,
                    requested_attrs: String,
                    requested_predicates: String,
                    revocation_details: String,
                    name: String) -> VcxResult<u32> {
    let verifier = Verifier::create(source_id, requested_attrs, requested_predicates, revocation_details, name)?;
    PROOF_MAP.add(verifier)
        .or(Err(VcxError::from(VcxErrorKind::CreateProof)))
}

fn apply_agent_info(proof: &mut Proof, agent_info: &MyAgentInfo) {
    proof.my_did = agent_info.my_pw_did.clone();
    proof.my_vk = agent_info.my_pw_vk.clone();
    proof.their_did = agent_info.their_pw_did.clone();
    proof.their_vk = agent_info.their_pw_vk.clone();
    proof.agent_did = agent_info.pw_agent_did.clone();
    proof.agent_vk = agent_info.pw_agent_vk.clone();
}

pub fn is_valid_handle(handle: u32) -> bool {
    PROOF_MAP.has_handle(handle)
}

pub fn update_state(handle: u32, message: Option<String>, connection_handle: Option<u32>) -> VcxResult<u32> {
    PROOF_MAP.get_mut(handle, |proof| {
        proof.update_state(message.as_ref().map(String::as_str), connection_handle)?;
        Ok(proof.state())
    })
}

pub fn get_state(handle: u32) -> VcxResult<u32> {
    PROOF_MAP.get(handle, |proof| {
        Ok(proof.state())
    })
}

pub fn get_proof_state(handle: u32) -> VcxResult<u32> {
    PROOF_MAP.get(handle, |proof| {
        Ok(proof.presentation_status())
    })
}

pub fn release(handle: u32) -> VcxResult<()> {
    PROOF_MAP.release(handle).or(Err(VcxError::from(VcxErrorKind::InvalidProofHandle)))
}

pub fn release_all() {
    PROOF_MAP.drain().ok();
}

pub fn to_string(handle: u32) -> VcxResult<String> {
    PROOF_MAP.get(handle, |proof| {
        serde_json::to_string(&Proofs::V3(proof.clone()))
            .map_err(|err| VcxError::from_msg(VcxErrorKind::InvalidState, format!("cannot serialize Proof proofect: {:?}", err)))
    })
}

pub fn get_source_id(handle: u32) -> VcxResult<String> {
    PROOF_MAP.get(handle, |proof| {
        Ok(proof.get_source_id())
    })
}

pub fn from_string(proof_data: &str) -> VcxResult<u32> {
    let proof: Proofs = serde_json::from_str(proof_data)
        .map_err(|err| VcxError::from_msg(VcxErrorKind::InvalidJson, format!("cannot deserialize Proofs proofect: {:?}", err)))?;

    match proof {
        Proofs::V3(proof) => PROOF_MAP.add(proof),
        _ => Err(VcxError::from_msg(VcxErrorKind::InvalidJson, "Found proof of unsupported version"))
    } 
}

pub fn generate_proof_request_msg(handle: u32) -> VcxResult<String> {
    PROOF_MAP.get_mut(handle, |proof| {
        proof.generate_presentation_request_msg()
    })
}

pub fn send_proof_request(handle: u32, connection_handle: u32) -> VcxResult<u32> {
    PROOF_MAP.get_mut(handle, |proof| {
        proof.send_presentation_request(connection_handle)?;
        Ok(error::SUCCESS.code_num)
    })
}


fn parse_proof_payload(payload: &str) -> VcxResult<ProofMessage> {
    let my_credential_req = ProofMessage::from_str(&payload)
        .map_err(|err| VcxError::from_msg(VcxErrorKind::InvalidJson, format!("Cannot deserialize ProofMessage: {}", err)))?;
    Ok(my_credential_req)
}

pub fn get_proof(handle: u32) -> VcxResult<String> {
    PROOF_MAP.get(handle, |proof| {
        proof.get_presentation()
    })
}

// TODO: This doesnt feel like it should be here (maybe utils?)
pub fn generate_nonce() -> VcxResult<String> {
    let mut bn = BigNum::new().map_err(|err| VcxError::from_msg(VcxErrorKind::EncodeError, format!("Cannot generate nonce: {}", err)))?;

    BigNumRef::rand(&mut bn, LARGE_NONCE as i32, openssl::bn::MsbOption::MAYBE_ZERO, false)
        .map_err(|err| VcxError::from_msg(VcxErrorKind::EncodeError, format!("Cannot generate nonce: {}", err)))?;
    Ok(bn.to_dec_str()
        .map_err(|err| VcxError::from_msg(VcxErrorKind::EncodeError, format!("Cannot generate nonce: {}", err)))?.to_string())
}

#[cfg(test)]
pub mod tests {
    
    use connection::tests::build_test_connection_inviter_requested;
    
    use utils::devsetup::*;
    use utils::httpclient::{HttpClientMockResponse};
    
    use utils::mockdata::mockdata_proof;
    
    use v3::handlers::proof_presentation::verifier::verifier::Verifier;
    

    use super::*;
    

    fn default_agent_info(connection_handle: Option<u32>) -> MyAgentInfo {
        if let Some(h) = connection_handle { get_agent_info().unwrap().pw_info(h).unwrap() } else {
            MyAgentInfo {
                my_pw_did: Some("GxtnGN6ypZYgEqcftSQFnC".to_string()),
                my_pw_vk: Some(VERKEY.to_string()),
                their_pw_did: Some(DID.to_string()),
                their_pw_vk: Some(VERKEY.to_string()),
                pw_agent_did: Some(DID.to_string()),
                pw_agent_vk: Some(VERKEY.to_string()),
                agent_did: DID.to_string(),
                agent_vk: VERKEY.to_string(),
                agency_did: DID.to_string(),
                agency_vk: VERKEY.to_string(),
                version: None,
                connection_handle,
            }
        }
    }

    pub fn create_default_proof_old(state: Option<VcxStateType>, proof_state: Option<ProofStateType>, connection_handle: Option<u32>) -> Proof {
        let agent_info = if let Some(h) = connection_handle {
            get_agent_info().unwrap().pw_info(h).unwrap()
        } else { default_agent_info(connection_handle) };
        let mut proof = Proof {
            source_id: "12".to_string(),
            msg_uid: String::from("1234"),
            ref_msg_id: String::new(),
            requested_attrs: String::from("[]"),
            requested_predicates: String::from("[]"),
            state: state.unwrap_or(VcxStateType::VcxStateOfferSent),
            proof_state: proof_state.unwrap_or(ProofStateType::ProofUndefined),
            name: String::new(),
            version: String::from("1.0"),
            nonce: generate_nonce().unwrap(),
            my_did: None,
            my_vk: None,
            their_did: None,
            their_vk: None,
            agent_did: None,
            agent_vk: None,
            proof: None,
            proof_request: None,
            revocation_interval: RevocationInterval { from: None, to: None },
            thread: Some(Thread::new()),
        };
        apply_agent_info(&mut proof, &agent_info);
        proof
    }

    fn create_boxed_proof(state: Option<VcxStateType>, proof_state: Option<ProofStateType>, connection_handle: Option<u32>) -> Box<Proof> {
        Box::new(create_default_proof_old(state, proof_state, connection_handle))
    }

    fn create_default_proof() -> Verifier {
        let proof = Verifier::create("1".to_string(),
                                  REQUESTED_ATTRS.to_owned(),
                                  REQUESTED_PREDICATES.to_owned(),
                                  r#"{"support_revocation":false}"#.to_string(),
                                  "Optional".to_owned()).unwrap();
        // let handle = PROOF_MAP.add(proof).unwrap();
        return proof
    }

    fn progress_proof_to_final_state(proof: &mut Verifier, connection_handle: u32) {
        proof.send_presentation_request(connection_handle).unwrap();
        assert_eq!(proof.state(), VcxStateType::VcxStateOfferSent as u32);

        proof.update_state(Some(mockdata_proof::ARIES_PROOF_PRESENTATION), None).unwrap();
        assert_eq!(proof.state(), VcxStateType::VcxStateAccepted as u32);
    }

    #[test]
    #[cfg(feature = "general_test")]
    fn test_create_proof_succeeds() {
        let _setup = SetupStrictAriesMocks::init();

        create_proof("1".to_string(),
                     REQUESTED_ATTRS.to_owned(),
                     REQUESTED_PREDICATES.to_owned(),
                     r#"{"support_revocation":false}"#.to_string(),
                     "Optional".to_owned()).unwrap();
    }

    #[test]
    #[cfg(feature = "general_test")]
    fn test_revocation_details() {
        let _setup = SetupStrictAriesMocks::init();

        // No Revocation
        create_proof("1".to_string(),
                     REQUESTED_ATTRS.to_owned(),
                     REQUESTED_PREDICATES.to_owned(),
                     r#"{"support_revocation":false}"#.to_string(),
                     "Optional".to_owned()).unwrap();

        // Support Revocation Success
        let revocation_details = json!({
            "to": 1234,
        });
        create_proof("1".to_string(),
                     REQUESTED_ATTRS.to_owned(),
                     REQUESTED_PREDICATES.to_owned(),
                     revocation_details.to_string(),
                     "Optional".to_owned()).unwrap();
    }

    #[test]
    #[cfg(feature = "general_test")]
    fn test_nonce() {
        let _setup = SetupDefaults::init();

        let nonce = generate_nonce().unwrap();
        assert!(BigNum::from_dec_str(&nonce).unwrap().num_bits() < 81)
    }

    #[test]
    #[cfg(feature = "general_test")]
    fn test_to_string_succeeds() {
        let _setup = SetupStrictAriesMocks::init();

        let handle = create_proof("1".to_string(),
                                  REQUESTED_ATTRS.to_owned(),
                                  REQUESTED_PREDICATES.to_owned(),
                                  r#"{"support_revocation":false}"#.to_string(),
                                  "Optional".to_owned()).unwrap();
        let proof_string = to_string(handle).unwrap();
        let s: Value = serde_json::from_str(&proof_string).unwrap();
        assert_eq!(s["version"], V3_OBJECT_SERIALIZE_VERSION);
        assert!(s["data"]["verifier_sm"].is_object());
        assert!(!proof_string.is_empty());
    }

    #[test]
    #[cfg(feature = "general_test")]
    fn test_from_string_succeeds() {
        let _setup = SetupStrictAriesMocks::init();

        let handle = create_proof("1".to_string(),
                                  REQUESTED_ATTRS.to_owned(),
                                  REQUESTED_PREDICATES.to_owned(),
                                  r#"{"support_revocation":false}"#.to_string(),
                                  "Optional".to_owned()).unwrap();
        let proof_data = to_string(handle).unwrap();
        let _hnadle2 = from_string(&proof_data).unwrap();
        let proof_data2 = to_string(handle).unwrap();
        assert_eq!(proof_data, proof_data2);
    }

    #[test]
    #[cfg(feature = "general_test")]
    fn test_release_proof() {
        let _setup = SetupStrictAriesMocks::init();

        let handle = create_proof("1".to_string(),
                                  REQUESTED_ATTRS.to_owned(),
                                  REQUESTED_PREDICATES.to_owned(),
                                  r#"{"support_revocation":false}"#.to_string(),
                                  "Optional".to_owned()).unwrap();
        assert!(release(handle).is_ok());
        assert!(!is_valid_handle(handle));
    }

    #[test]
    #[cfg(feature = "general_test")]
    fn test_send_proof_request() {
        let _setup = SetupStrictAriesMocks::init();

        let connection_handle = build_test_connection_inviter_requested();

        let proof_handle = create_proof("1".to_string(),
                                        REQUESTED_ATTRS.to_owned(),
                                        REQUESTED_PREDICATES.to_owned(),
                                        r#"{"support_revocation":false}"#.to_string(),
                                        "Optional".to_owned()).unwrap();
        assert_eq!(send_proof_request(proof_handle, connection_handle).unwrap(), error::SUCCESS.code_num);
        assert_eq!(get_state(proof_handle).unwrap(), VcxStateType::VcxStateOfferSent as u32);
    }

    #[test]
    #[cfg(feature = "general_test")]
    fn test_get_proof_fails_with_no_proof() {
        let _setup = SetupStrictAriesMocks::init();

        let handle = create_proof("1".to_string(),
                                  REQUESTED_ATTRS.to_owned(),
                                  REQUESTED_PREDICATES.to_owned(),
                                  r#"{"support_revocation":false}"#.to_string(),
                                  "Optional".to_owned()).unwrap();
        assert!(is_valid_handle(handle));
        assert!(get_proof(handle).is_err())
    }

    #[test]
    #[cfg(feature = "general_test")]
    fn test_proof_update_state_v2() {
        let _setup = SetupStrictAriesMocks::init();

        let connection_handle = build_test_connection_inviter_requested();

        let mut proof = create_default_proof();
        proof.send_presentation_request(connection_handle).unwrap();
        assert_eq!(proof.state(), VcxStateType::VcxStateOfferSent as u32);

        ::connection::release(connection_handle);
        let connection_handle = build_test_connection_inviter_requested();

        proof.update_state(Some(mockdata_proof::ARIES_PROOF_PRESENTATION), Some(connection_handle)).unwrap();

        assert_eq!(proof.state(), VcxStateType::VcxStateAccepted as u32);
    }

    #[test]
    #[cfg(feature = "general_test")]
    fn test_update_state_with_message() {
        let _setup = SetupStrictAriesMocks::init();

        let connection_handle = build_test_connection_inviter_requested();

        let mut proof = create_default_proof();
        progress_proof_to_final_state(&mut proof, connection_handle);
    }

    #[test]
    #[cfg(feature = "general_test")]
    fn test_build_credential_defs_json_with_multiple_credentials() {
        let _setup = SetupStrictAriesMocks::init();

        let cred1 = CredInfo {
            schema_id: "schema_key1".to_string(),
            cred_def_id: "cred_def_key1".to_string(),
            rev_reg_id: None,
            timestamp: None,
        };
        let cred2 = CredInfo {
            schema_id: "schema_key2".to_string(),
            cred_def_id: "cred_def_key2".to_string(),
            rev_reg_id: None,
            timestamp: None,
        };
        let credentials = vec![cred1, cred2];
        let credential_json = Proof::build_credential_defs_json(&credentials).unwrap();

        let json: Value = serde_json::from_str(CRED_DEF_JSON).unwrap();
        let expected = json!({CRED_DEF_ID:json}).to_string();
        assert_eq!(credential_json, expected);
    }

    #[test]
    #[cfg(feature = "general_test")]
    fn test_build_schemas_json_with_multiple_schemas() {
        let _setup = SetupStrictAriesMocks::init();

        let cred1 = CredInfo {
            schema_id: "schema_key1".to_string(),
            cred_def_id: "cred_def_key1".to_string(),
            rev_reg_id: None,
            timestamp: None,
        };
        let cred2 = CredInfo {
            schema_id: "schema_key2".to_string(),
            cred_def_id: "cred_def_key2".to_string(),
            rev_reg_id: None,
            timestamp: None,
        };
        let credentials = vec![cred1, cred2];
        let schema_json = Proof::build_schemas_json(&credentials).unwrap();

        let json: Value = serde_json::from_str(SCHEMA_JSON).unwrap();
        let expected = json!({SCHEMA_ID:json}).to_string();
        assert_eq!(schema_json, expected);
    }

    #[test]
    #[cfg(feature = "general_test")]
    fn test_build_rev_reg_defs_json() {
        let _setup = SetupStrictAriesMocks::init();

        let cred1 = CredInfo {
            schema_id: "schema_key1".to_string(),
            cred_def_id: "cred_def_key1".to_string(),
            rev_reg_id: Some("id1".to_string()),
            timestamp: None,
        };
        let cred2 = CredInfo {
            schema_id: "schema_key2".to_string(),
            cred_def_id: "cred_def_key2".to_string(),
            rev_reg_id: Some("id2".to_string()),
            timestamp: None,
        };
        let credentials = vec![cred1, cred2];
        let rev_reg_defs_json = Proof::build_rev_reg_defs_json(&credentials).unwrap();

        let json: Value = serde_json::from_str(&rev_def_json()).unwrap();
        let expected = json!({REV_REG_ID:json}).to_string();
        assert_eq!(rev_reg_defs_json, expected);
    }

    #[test]
    #[cfg(feature = "general_test")]
    fn test_build_rev_reg_json() {
        let _setup = SetupStrictAriesMocks::init();

        let cred1 = CredInfo {
            schema_id: "schema_key1".to_string(),
            cred_def_id: "cred_def_key1".to_string(),
            rev_reg_id: Some("id1".to_string()),
            timestamp: Some(1),
        };
        let cred2 = CredInfo {
            schema_id: "schema_key2".to_string(),
            cred_def_id: "cred_def_key2".to_string(),
            rev_reg_id: Some("id2".to_string()),
            timestamp: Some(2),
        };
        let credentials = vec![cred1, cred2];
        let rev_reg_json = Proof::build_rev_reg_json(&credentials).unwrap();

        let json: Value = serde_json::from_str(REV_REG_JSON).unwrap();
        let expected = json!({REV_REG_ID:{"1":json}}).to_string();
        assert_eq!(rev_reg_json, expected);
    }

    #[test]
    #[cfg(feature = "general_test")]
    fn test_get_proof() {
        let _setup = SetupStrictAriesMocks::init();

        let connection_handle = build_test_connection_inviter_requested();
        let mut proof = create_default_proof();
        progress_proof_to_final_state(&mut proof, connection_handle);

        let handle = PROOF_MAP.add(proof).unwrap();
        let proof_str = get_proof(handle).unwrap();
        // TODO: Why don't these equal? Parse compare values?
        // assert_eq!(&proof_str, mockdata_proof::ARIES_PROOF_PRESENTATION);
    }

    #[test]
    #[cfg(feature = "general_test")]
    fn test_release_all() {
        let _setup = SetupStrictAriesMocks::init();

        let h1 = create_proof("1".to_string(), REQUESTED_ATTRS.to_owned(), REQUESTED_PREDICATES.to_owned(), r#"{"support_revocation":false}"#.to_string(), "Optional".to_owned()).unwrap();
        let h2 = create_proof("1".to_string(), REQUESTED_ATTRS.to_owned(), REQUESTED_PREDICATES.to_owned(), r#"{"support_revocation":false}"#.to_string(), "Optional".to_owned()).unwrap();
        let h3 = create_proof("1".to_string(), REQUESTED_ATTRS.to_owned(), REQUESTED_PREDICATES.to_owned(), r#"{"support_revocation":false}"#.to_string(), "Optional".to_owned()).unwrap();
        let h4 = create_proof("1".to_string(), REQUESTED_ATTRS.to_owned(), REQUESTED_PREDICATES.to_owned(), r#"{"support_revocation":false}"#.to_string(), "Optional".to_owned()).unwrap();
        let h5 = create_proof("1".to_string(), REQUESTED_ATTRS.to_owned(), REQUESTED_PREDICATES.to_owned(), r#"{"support_revocation":false}"#.to_string(), "Optional".to_owned()).unwrap();
        release_all();
        assert_eq!(release(h1).unwrap_err().kind(), VcxErrorKind::InvalidProofHandle);
        assert_eq!(release(h2).unwrap_err().kind(), VcxErrorKind::InvalidProofHandle);
        assert_eq!(release(h3).unwrap_err().kind(), VcxErrorKind::InvalidProofHandle);
        assert_eq!(release(h4).unwrap_err().kind(), VcxErrorKind::InvalidProofHandle);
        assert_eq!(release(h5).unwrap_err().kind(), VcxErrorKind::InvalidProofHandle);
    }

    #[test]
    #[cfg(feature = "general_test")]
    fn test_send_proof_request_can_be_retried() {
        let _setup = SetupStrictAriesMocks::init();
        settings::set_config_value(settings::CONFIG_PROTOCOL_TYPE, "4.0");

        let handle_conn = build_test_connection_inviter_requested();

        let handle_proof = create_proof("1".to_string(),
                                        REQUESTED_ATTRS.to_owned(),
                                        REQUESTED_PREDICATES.to_owned(),
                                        r#"{"support_revocation":false}"#.to_string(),
                                        "Optional".to_owned()).unwrap();
        let _request = generate_proof_request_msg(handle_proof).unwrap();
        assert_eq!(get_state(handle_proof).unwrap(), VcxStateType::VcxStateInitialized as u32);

        HttpClientMockResponse::set_next_response(VcxResult::Err(VcxError::from_msg(VcxErrorKind::IOError, "Sending message timeout.")));
        assert_eq!(send_proof_request(handle_proof, handle_conn).unwrap_err().kind(), VcxErrorKind::IOError);
        assert_eq!(get_state(handle_proof).unwrap(), VcxStateType::VcxStateInitialized as u32);

        // Retry sending proof request
        assert_eq!(send_proof_request(handle_proof, handle_conn).unwrap(), 0);
        assert_eq!(get_state(handle_proof).unwrap(), VcxStateType::VcxStateOfferSent as u32);
    }

    #[test]
    #[cfg(feature = "general_test")]
    fn test_proof_accepted() {
        let _setup = SetupStrictAriesMocks::init();

        let handle_conn = build_test_connection_inviter_requested();

        let handle_proof = create_proof("1".to_string(),
                                        REQUESTED_ATTRS.to_owned(),
                                        REQUESTED_PREDICATES.to_owned(),
                                        r#"{"support_revocation":false}"#.to_string(),
                                        "Optional".to_owned()).unwrap();
        let _request = generate_proof_request_msg(handle_proof).unwrap();
        send_proof_request(handle_proof, handle_conn).unwrap();
        update_state(handle_proof, Some(mockdata_proof::ARIES_PROOF_PRESENTATION.to_string()), Some(handle_conn)).unwrap();
        assert_eq!(::proof::get_state(handle_proof).unwrap(), VcxStateType::VcxStateAccepted as u32);
    }

    #[test]
    #[cfg(feature = "general_test")]
    fn test_proof_errors() {
        SetupLibraryWallet::init();
        let _setup = SetupStrictAriesMocks::init();

        let connection_handle = build_test_connection_inviter_requested();

        let proof = create_default_proof();
        let proof_handle = PROOF_MAP.add(proof).unwrap();

        let bad_handle = 100000;
        let empty = r#""#;

        assert_eq!(send_proof_request(bad_handle, connection_handle).unwrap_err().kind(), VcxErrorKind::InvalidHandle);
        assert_eq!(get_proof_state(proof_handle).unwrap(), 0);
        assert_eq!(create_proof("my source id".to_string(),
                                empty.to_string(),
                                "{}".to_string(),
                                r#"{"support_revocation":false}"#.to_string(),
                                "my name".to_string()).unwrap_err().kind(), VcxErrorKind::InvalidJson);
        assert_eq!(to_string(bad_handle).unwrap_err().kind(), VcxErrorKind::InvalidHandle);
        assert_eq!(get_source_id(bad_handle).unwrap_err().kind(), VcxErrorKind::InvalidHandle);
        assert_eq!(from_string(empty).unwrap_err().kind(), VcxErrorKind::InvalidJson);
    }

    #[cfg(feature = "pool_tests")]
    #[cfg(feature = "to_restore")]
    #[test]
    fn test_proof_validate_attribute() {
        let _setup = SetupLibraryWalletPoolZeroFees::init();

        let (_, _, proof_req, proof_json) = ::utils::libindy::anoncreds::tests::create_proof();

        let mut proof_req_obj = ProofRequestMessage::create();

        proof_req_obj.proof_request_data = serde_json::from_str(&proof_req).unwrap();

        let mut proof_msg = ProofMessage::new();
        let mut proof = create_boxed_proof(None, None, None);
        proof.proof_request = Some(proof_req_obj);

        // valid proof_obj
        {
            proof_msg.libindy_proof = proof_json.clone();
            proof.proof = Some(proof_msg);

            let _rc = proof.proof_validation().unwrap();
            assert_eq!(proof.proof_state, ProofStateType::ProofValidated);
        }

        let mut proof_obj: serde_json::Value = serde_json::from_str(&proof_json).unwrap();

        // change Raw value
        {
            let mut proof_msg = ProofMessage::new();
            proof_obj["requested_proof"]["revealed_attrs"]["address1_1"]["raw"] = json!("Other Value");
            let proof_json = serde_json::to_string(&proof_obj).unwrap();

            proof_msg.libindy_proof = proof_json;
            proof.proof = Some(proof_msg);

            let rc = proof.proof_validation();
            rc.unwrap_err();
            assert_eq!(proof.get_proof_state(), ProofStateType::ProofInvalid as u32);
        }

        // change Encoded value
        {
            let mut proof_msg = ProofMessage::new();
            proof_obj["requested_proof"]["revealed_attrs"]["address1_1"]["encoded"] = json!("1111111111111111111111111111111111111111111111111111111111");
            let proof_json = serde_json::to_string(&proof_obj).unwrap();

            proof_msg.libindy_proof = proof_json;
            proof.proof = Some(proof_msg);

            let rc = proof.proof_validation();
            rc.unwrap_err(); //FIXME check error code also
            assert_eq!(proof.get_proof_state(), ProofStateType::ProofInvalid as u32);
        }
    }
}
