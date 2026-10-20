//! Transport observations only: this fixture does not implement a mint or proof locks.
use cdk_daraja_processor::proto::{
    self,
    cdk_payment_processor_client::CdkPaymentProcessorClient,
    cdk_payment_processor_server::{CdkPaymentProcessor, CdkPaymentProcessorServer},
};
use tokio::{net::TcpListener, sync::mpsc};
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
use tonic::{Request, Response, Status};

#[derive(Clone)]
struct Fixture;

#[tonic::async_trait]
impl CdkPaymentProcessor for Fixture {
    async fn get_settings(
        &self,
        _: Request<proto::EmptyRequest>,
    ) -> Result<Response<proto::SettingsResponse>, Status> {
        Ok(Response::new(proto::SettingsResponse {
            unit: "kes".into(),
            custom: [("mpesa".into(), "{}".into())].into(),
            ..Default::default()
        }))
    }
    async fn create_payment(
        &self,
        _: Request<proto::CreatePaymentRequest>,
    ) -> Result<Response<proto::CreatePaymentResponse>, Status> {
        Err(Status::unimplemented("outside fixture"))
    }
    async fn get_payment_quote(
        &self,
        _: Request<proto::PaymentQuoteRequest>,
    ) -> Result<Response<proto::PaymentQuoteResponse>, Status> {
        Err(Status::unimplemented("outside fixture"))
    }
    async fn make_payment(
        &self,
        request: Request<proto::MakePaymentRequest>,
    ) -> Result<Response<proto::MakePaymentResponse>, Status> {
        if request.into_inner().unit == "error" {
            return Err(Status::unavailable("synthetic transport error"));
        }
        Ok(Response::new(pending()))
    }
    async fn check_incoming_payment(
        &self,
        _: Request<proto::CheckIncomingPaymentRequest>,
    ) -> Result<Response<proto::CheckIncomingPaymentResponse>, Status> {
        Err(Status::unimplemented("outside fixture"))
    }
    async fn check_outgoing_payment(
        &self,
        _: Request<proto::CheckOutgoingPaymentRequest>,
    ) -> Result<Response<proto::MakePaymentResponse>, Status> {
        Ok(Response::new(pending()))
    }
    type WaitPaymentEventStream = ReceiverStream<Result<proto::PaymentEventResponse, Status>>;
    async fn wait_payment_event(
        &self,
        _: Request<proto::EmptyRequest>,
    ) -> Result<Response<Self::WaitPaymentEventStream>, Status> {
        let (sender, receiver) = mpsc::channel(1);
        sender
            .send(Err(Status::unavailable("synthetic stream disconnect")))
            .await
            .unwrap();
        Ok(Response::new(ReceiverStream::new(receiver)))
    }
}

fn pending() -> proto::MakePaymentResponse {
    proto::MakePaymentResponse {
        payment_identifier: Some(proto::PaymentIdentifier {
            r#type: proto::PaymentIdentifierType::CustomId as i32,
            value: Some(proto::payment_identifier::Value::Id(
                "synthetic-operation".into(),
            )),
        }),
        status: proto::QuoteState::Pending as i32,
        ..Default::default()
    }
}

async fn server() -> (
    CdkPaymentProcessorClient<tonic::transport::Channel>,
    tokio::task::JoinHandle<()>,
    String,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(CdkPaymentProcessorServer::new(Fixture))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let client = CdkPaymentProcessorClient::connect(format!("http://{address}"))
        .await
        .unwrap();
    (client, task, format!("http://{address}"))
}

#[tokio::test]
async fn pending_error_disconnect_and_fresh_server_preserve_wire_distinctions() {
    let (mut client, task, address) = server().await;
    let settings = client
        .get_settings(proto::EmptyRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(settings.unit, "kes");
    assert!(settings.custom.contains_key("mpesa"));
    let response = client
        .make_payment(proto::MakePaymentRequest {
            unit: "kes".into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(response, pending());
    assert!(response.payment_proof.is_none());
    let error = client
        .make_payment(proto::MakePaymentRequest {
            unit: "error".into(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::Unavailable);
    let mut stream = client
        .wait_payment_event(proto::EmptyRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        stream.message().await.unwrap_err().code(),
        tonic::Code::Unavailable
    );
    task.abort();
    let _ = task.await;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        CdkPaymentProcessorClient::connect(address),
    )
    .await;
    assert!(matches!(result, Ok(Err(_))));
    let (mut restarted, task, _) = server().await;
    assert_eq!(
        restarted
            .check_outgoing_payment(proto::CheckOutgoingPaymentRequest::default())
            .await
            .unwrap()
            .into_inner(),
        pending()
    );
    task.abort();
}
