use anyhow::anyhow;
use futures_util::stream::StreamExt;
use matrix_sdk::{
    Client,
    encryption::verification::{
        Emoji, SasState, SasVerification, Verification, VerificationRequest,
        VerificationRequestState, format_emojis,
    },
    ruma::{DeviceId, UserId, events::key::verification::VerificationMethod},
};
use tokio::runtime::Handle;

use crate::{
    init::singletons::{get_client, get_event_bridge, get_verification_response_receiver_lock},
    models::events::{EmitEvent, MatrixVerificationEmojis},
};

async fn wait_for_confirmation(sas: SasVerification, emoji: [Emoji; 7]) -> anyhow::Result<()> {
    let payload = MatrixVerificationEmojis::new(format_emojis(emoji));
    let event_bridge = get_event_bridge()?;

    event_bridge.emit(EmitEvent::VerificationStart(payload));

    let mut receiver = get_verification_response_receiver_lock().await?;

    if let Some(msg) = receiver.recv().await {
        match msg.confirmed {
            true => sas.confirm().await.unwrap(),
            false => sas.cancel().await.unwrap(),
        }
    }

    Ok(())
}

async fn print_devices(user_id: &UserId, client: &Client) {
    println!("Devices of user {user_id}");

    for device in client
        .encryption()
        .get_user_devices(user_id)
        .await
        .unwrap()
        .devices()
    {
        if device.device_id()
            == client
                .device_id()
                .expect("We should be logged in now and know our device id")
        {
            continue;
        }

        println!(
            "   {:<10} {:<30} {:<}",
            device.device_id(),
            device.display_name().unwrap_or("-"),
            if device.is_verified() { "✅" } else { "❌" }
        );
    }
}

async fn sas_verification_handler(client: Client, sas: SasVerification) {
    println!(
        "Starting verification with {} {}",
        &sas.other_device().user_id(),
        &sas.other_device().device_id()
    );
    print_devices(sas.other_device().user_id(), &client).await;
    sas.accept().await.unwrap();

    let mut stream = sas.changes();

    while let Some(state) = stream.next().await {
        match state {
            SasState::KeysExchanged {
                emojis,
                decimals: _,
            } => {
                Handle::current().spawn(wait_for_confirmation(
                    sas.clone(),
                    emojis
                        .expect("We only support verifications using emojis")
                        .emojis,
                ));
            }
            SasState::Done { .. } => {
                let device = sas.other_device();

                println!(
                    "Successfully verified device {} {} {:?}",
                    device.user_id(),
                    device.device_id(),
                    device.local_trust_state()
                );

                print_devices(sas.other_device().user_id(), &client).await;

                break;
            }
            SasState::Cancelled(cancel_info) => {
                println!(
                    "The verification has been cancelled, reason: {}",
                    cancel_info.reason()
                );

                break;
            }
            SasState::Created { .. }
            | SasState::Started { .. }
            | SasState::Accepted { .. }
            | SasState::Confirmed => (),
        }
    }
}

pub async fn request_verification_handler(client: Client, request: VerificationRequest) {
    println!(
        "Accepting verification request from {}",
        request.other_user_id(),
    );
    request
        .accept()
        .await
        .expect("Can't accept verification request");

    let mut stream = request.changes();

    while let Some(state) = stream.next().await {
        match state {
            VerificationRequestState::Created { .. }
            | VerificationRequestState::Requested { .. }
            | VerificationRequestState::Ready { .. } => (),
            VerificationRequestState::Transitioned { verification } => {
                // We only support SAS verification.
                if let Verification::SasV1(s) = verification {
                    Handle::current().spawn(sas_verification_handler(client, s));
                    break;
                }
            }
            VerificationRequestState::Done | VerificationRequestState::Cancelled(_) => break,
        }
    }
}

pub async fn verify_device(user_id: &UserId, device_id: &DeviceId) -> anyhow::Result<()> {
    let client = get_client().expect("Client should be defined at this state");
    let device_option = client
        .encryption()
        .get_device(user_id, device_id)
        .await
        .map_err(|e| anyhow!(e))?;

    let verification_methods = vec![VerificationMethod::SasV1];

    let request = if let Some(device) = device_option {
        device
            .request_verification_with_methods(verification_methods)
            .await?
    } else {
        return Err(anyhow!("The provided device ID is not found"));
    };

    let mut stream = request.changes();

    while let Some(state) = stream.next().await {
        match state {
            VerificationRequestState::Created { .. }
            | VerificationRequestState::Requested { .. }
            | VerificationRequestState::Transitioned { .. } => (),
            VerificationRequestState::Ready {
                our_methods: _,
                other_device_data: _,
                their_methods,
            } => {
                if their_methods.contains(&VerificationMethod::SasV1) {
                    if let Some(sas) = request.start_sas().await? {
                        Handle::current().spawn(sas_verification_handler(client, sas));
                        break;
                    };
                } else {
                    request.cancel().await?
                }
            }
            VerificationRequestState::Done | VerificationRequestState::Cancelled(_) => break,
        }
    }

    Ok(())
}
