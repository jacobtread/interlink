//! Name reserved

mod ctx;
mod envelope;
mod link;
mod message;
mod service;

#[cfg(test)]
mod test {
    use tokio::sync::mpsc;

    use crate::{
        link::Link,
        service::{self, Service},
    };

    pub struct TestService {
        pub test: String,
    }

    impl Service for TestService {}

    #[test]
    fn test() {
        let (tx, rx) = mpsc::unbounded_channel();
        let link = Link { tx } as Link<TestService>;

        link.wait(|service, ctx| {
            Box::pin(async move {
                let s = service.test.clone();
            })
        });
        println!("Test");
    }
}
