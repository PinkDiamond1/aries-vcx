use crate::did_doc::DidDoc;
use crate::messages::connection::invite::Invitation;
use crate::messages::connection::request::Request;
use crate::protocols::connection::invitee::states::requested::RequestedState;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InvitedState {
    pub invitation: Invitation,
}

impl From<(InvitedState, Request)> for RequestedState {
    fn from((state, request): (InvitedState, Request)) -> RequestedState {
        trace!("ConnectionInvitee: transit state from InvitedState to RequestedState");
        RequestedState {
            request,
            did_doc: DidDoc::from(state.invitation),
        }
    }
}
