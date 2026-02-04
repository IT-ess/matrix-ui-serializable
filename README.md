# matrix-ui-serializable

This library creates an even higher level abstraction of a Matrix client than [matrix_sdk_ui](https://docs.rs/matrix-sdk-ui/) does, by exposing serializable structs representing the RoomsList and Rooms.
This lib is then used by adapters like [tauri-plugin-matrix-svelte](https://github.com/IT-ess/tauri-plugin-matrix-svelte) to bridge the serialized structs to a frontend of your choice. 

# Features

This lib is mostly a port of some great bits from [Robrix](https://github.com/project-robius/robrix), with few adaptations. The main Matrix related features supported are :

- Basics (login, sync, room list, creating rooms, text messages)
- Sending and receiving media or audio messages
- Replying to, reacting, editing, or redacting a message
- Basic threads support
- Device verification and recovery
- OS & Mobile Push notifications (requires a [Sygnal](https://github.com/element-hq/sygnal) gateway)

**These features are showcased in the example [matrix-svelte-client](https://github.com/IT-ess/tauri-plugin-matrix-svelte/tree/main/example/matrix-svelte-client)**.


# Architecture

The goal of this lib is to provide an even higher abstraction than matrix-sdk-ui does. Mostly, it abstracts each piece of the frontend state in a reactive serializable store.

**This lib is intended to be frontend agnostic**, and must be used with an "adapter" (such as [tauri-plugin-matrix-svelte](https://github.com/IT-ess/tauri-plugin-matrix-svelte)). The adapter must implement some functions that will serialize the new state, pass events, or expose commands.

Thus, this lib follows more or less the MVVM architectural pattern, this lib + adapter being the Model & Viewmodel, and your frontend the View.


# Usage

To use this lib you must create your own "adapter" (or use an existing one). The adapter must handle the three means of communication between frontend and backend : **commands**, **events** and **state updates**.

## State updates

Whenever the state of an abstracted object (a Matrix room for instance), the backend must update the frontend accordingly. This is done through functions passed at the initizalization of this lib, the *updaters*, which role is to translate the backend state into frontend state. These updaters have access to the backend state and can serialize it (all states have already the Serialize trait) if it is required by your frontend.

## Commands

Actions that return a response when invoked. These commands must be imported from the `commands` module and exposed to your frontend. Not all commands should be implemented, but some are required if you want to login for instance. More details about commands [here](https://docs.rs/matrix-ui-serializable/latest/matrix_ui_serializable/commands/index.html).

## Events

Backend and frontend also communicate through events, going from the backend to frontend or the other way.

### Outgoing events (lib -> adapter -> frontend)

Backend outgoing events work with a tokio broadcaster. The *receiver* part of the broadcaster channel is returned by the `init` function of this lib to the adapter which can then listen and forward events to the frontend. Event payloads are also serializable. More details about outgoing events [here](https://docs.rs/matrix-ui-serializable/latest/matrix_ui_serializable/models/events/enum.EmitEvent.html).

### Incoming events (lib <- adapter <- frontend)

Frontend events cannot be listen directly by the lib, so the adapter must forward them to the lib through a tokio channel. The receiver part of this channel must be passed at lib initialization (see LibConfig). More details about incoming events [here](https://docs.rs/matrix-ui-serializable/latest/matrix_ui_serializable/struct.EventReceivers.html).

## Initializing the lib

The lib init is handled by a single `init` function that accepts a `LibConfig` struct. This struct is constructed with the `updaters` and `event_receivers` mentionned earlier plus those objects :

### Session option

To be OS agnostic, the session storage (and encryption) must be handled by the adapter.
Thus if an existing session is found, pass it to the lib, and the matrix client will be restored automatically. If not, pass `None`, and handle login with the login commands.

### App data directory

The Matrix DB and app files will be stored there. Just provide a `PathBuf` to this dir.

### Oauth client variables

`oauth_client_uri` and `oauth_redirect_uri` that will be used in case of an OAuth login flow.

# Contributing
This project is opened to all kinds of contributions. I'm aware that the [documentation](https://docs.rs/matrix-ui-serializable) isn't exhaustive and I do not have enough time to make it so. I can still [answer some questions](#chat-about-this-project) if needed !

# Possible improvements

The approach of this wrapper is quite simple, we make things serializable and we call the frontend updaters every time needed. Obviously, there is some room for performance improvements, since serialization costs a lot.
However, the consumers of this lib still have the choice not to serialize the crates on each change, since they only have to implement a callback with the updated Rust structs (not yet serialized).
The serialization feature of the RoomsList and RoomScreen could then be feature flagged, depending on the update method of the adapter. 

# Chat about this project

Join this [Matrix room](https://matrix.to/#/#matrix-ui-serializable:matrix.org) if you have questions about this project !

# Credits

Huge thanks to [Kevin Boos](https://github.com/kevinaboos) and the [Robius project](https://github.com/project-robius) team for their awesome work on the [Robrix](https://github.com/project-robius/robrix) client that inspired me for this different implementation.
Also, shoutout to the Element and Matrix.org team for the wonderful and *blazingly fast* matrix-rust-sdk.
