use cdk_daraja_processor::{FILE_DESCRIPTOR_SET, proto};
use prost::Message;

#[test]
fn pinned_service_exposes_upstream_event_stream() {
    let descriptor = prost_types::FileDescriptorSet::decode(FILE_DESCRIPTOR_SET).unwrap();
    let file = descriptor
        .file
        .iter()
        .find(|file| file.package.as_deref() == Some("cdk_payment_processor"))
        .unwrap();
    let service = file
        .service
        .iter()
        .find(|service| service.name.as_deref() == Some("CdkPaymentProcessor"))
        .unwrap();
    let methods: Vec<_> = service
        .method
        .iter()
        .map(|method| {
            (
                method.name.as_deref().unwrap(),
                method.server_streaming.unwrap_or(false),
            )
        })
        .collect();
    assert_eq!(
        methods,
        [
            ("GetSettings", false),
            ("CreatePayment", false),
            ("GetPaymentQuote", false),
            ("MakePayment", false),
            ("CheckIncomingPayment", false),
            ("CheckOutgoingPayment", false),
            ("WaitPaymentEvent", true),
        ]
    );
}

#[test]
fn custom_quote_correlation_survives_protobuf_roundtrip() {
    let request = proto::CreatePaymentRequest {
        options: Some(proto::IncomingPaymentOptions {
            options: Some(proto::incoming_payment_options::Options::Custom(
                proto::CustomIncomingPaymentOptions {
                    amount: Some(proto::AmountMessage {
                        value: 123,
                        unit: "kes".into(),
                    }),
                    quote_id: "synthetic-quote".into(),
                    extra_json: Some(r#"{"method":"mpesa"}"#.into()),
                    ..Default::default()
                },
            )),
        }),
    };
    let decoded = proto::CreatePaymentRequest::decode(request.encode_to_vec().as_slice()).unwrap();
    assert_eq!(decoded, request);
}
