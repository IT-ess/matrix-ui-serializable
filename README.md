# matrix-ui-serializable

This library creates an even higher abstraction of a Matrix client than [matrix_sdk_ui](https://docs.rs/matrix-sdk-ui/) does, by exposing high-level serializable components states (a Matrix room for instance).

# Features

This lib is mostly a port of [Robrix](https://github.com/project-robius/robrix) backend, with few adaptations. The current main features are :

- Basics (login, sync, room list, creating a room, room actions, basic message like structs)
- 🔥 High level serializable states. For instance, each room has its own serializable state (which includes its timeline). These states can then be translated to the frontend into native reactive stores for better integration.
- Client emoji verification of user's devices (outgoing or incoming)
- OS & Mobile Push notifications

For a full implementation example of this lib in a client, see [matrix-svelte-client](https://github.com/IT-ess/tauri-plugin-matrix-svelte/tree/main/example/matrix-svelte-client).


# Architecture

The goal of this lib is to provide an even higher abstraction than matrix-sdk-ui does. Mostly, it abstracts each piece of the frontend state in a reactive serializable store.

**This lib is intended to be frontend agnostic**, and must be used with an "adapter" (such as [tauri-plugin-matrix-svelte](https://github.com/IT-ess/tauri-plugin-matrix-svelte)) that acts as a translation layer between the Rust state, and the frontend state. WASM and UniFFI bindings are yet not supported but they might be in the future.

Thus, this lib follows more or less the MVVM architectural pattern, this lib + adapter being the Model & Viewmodel, and your frontend the View.


# Usage
<!--To use this lib you must call the `matrix_ui_serializable::init` function with a valid `LibConfig` struct. This struct contains all required mechanisms your adapter must implement to update the frontend state :-->

To use this lib you must create your own "adapter" (or use an existing one). The adapter must handle the three means of communication between frontend and backend : **commands**, **events** and **state updates**.

## State updates

Whenever the state of an abstracted object (a Matrix room for instance), the backend must update the frontend accordingly. This is done through functions passed at the initizalization of this lib, the *updaters*, which role is to translate the backend state into frontend state. These updaters have access to the backend state and can serialize it (all states have already the Serialize trait) if it is required by your frontend.

## Commands

Actions that return a response when invoked. These commands must be imported from the `commands` module and exposed to your frontend. At least the `login_and_create_new_session` must be exposed to log in. More details about commands here.

## Events

Backend and frontend also communicate through events, going from the backend to frontend or the other way.

### Outgoing events (lib -> adapter -> frontend)

Backend outgoing events work with a tokio broadcaster. The *receiver* part of the broadcaster channel is returned by the `init` function of this lib to the adapter which can then listen and forward events to the frontend. Event payloads are also serializable. More details about outgoing events here.

### Incoming events (lib <- adapter <- frontend)

Frontend events cannot be listen directly by the lib, so the adapter must forward them to the lib through a tokio channel. The receiver part of this channel must be passed at lib initialization (see LibConfig). More details about incoming events here.


<!--
### Updaters

Those are the functions that are called whenever the state of a given store changes. They have access to the Rust state and can serialize this state if needed (to pass it to a javascript frontend for instance). More details about each updater here.

### Event receivers



### Session option

To be OS agnostic, the session storage (and encryption) must be handled by the adapter. If an existing session is found, pass it, and the matrix client will be restored automatically. If not, -->
