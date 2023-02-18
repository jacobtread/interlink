//! Name reserved

pub mod ctx;
mod envelope;
pub mod link;
pub mod msg;
pub mod prelude;
pub mod service;

#[cfg(test)]
mod test {
    use crate::{msg::MessageResponse, prelude::*};
    use std::time::Duration;
    use tokio::time::sleep;

    pub struct TestService {
        pub test: String,
    }

    impl Service for TestService {}

    struct TestMessage;

    impl Message for TestMessage {
        type Response = String;
    }

    impl Handler<TestMessage> for TestService {
        type Response = MessageResponse<String>;

        fn handle(
            &mut self,
            _msg: TestMessage,
            _ctx: &mut crate::ctx::ServiceContext<Self>,
        ) -> MessageResponse<String> {
            MessageResponse("got response from TestService handler".to_string())
        }
    }
    #[tokio::test]
    async fn test() {
        let link = TestService {
            test: "Welcome to linking".to_string(),
        }
        .start();
        link.do_wait(|service, _ctx| {
            Box::pin(async move {
                println!("Waiting async using the TestService processor");
                sleep(Duration::from_millis(1000)).await;
                println!("{}", service.test)
            })
        })
        .ok();

        let resp = link.send(TestMessage).await.unwrap();
        println!("GOT RESPONSE: {}", resp);

        println!("Test");
    }
}
