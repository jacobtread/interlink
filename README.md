# Interlink

> This project is in its very early infantcy but im creating it to 
> extend an existing project of mine that is already using a similar
> structure but that is not as well layed out

## What 

Interlink is a framework for structuring and communicating between sets of logic using messages
similar to the Actix actors system.

Logic is seperated into "Services" which can be communicated with using "Links" that are able
to send messages to services along with modifying their state. 

Services are spawned on their own tokio task and each have a processing loop which awaits messages
from the Links and also can process "wait" futures which require a mutable borrow over the service state