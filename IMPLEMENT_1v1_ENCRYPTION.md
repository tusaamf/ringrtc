# Implementing Media Encryption for 1:1 Calls

## Overview
To add end-to-end encryption support for 1:1 calls (like group calls already have), follow these steps:

## Step 1: Add Import for Frame Crypto

**File:** `src/rust/src/core/connection.rs`

In the `use crate::` section (around line 26-56), add the frame_crypto import:

```rust
use crate::{
    common::{
        // ... existing imports
    },
    core::{
        // ... existing imports
        crypto as frame_crypto,  // <-- ADD THIS
    },
    // ... rest of imports
};
```

## Step 2: Add frame_crypto_context Field to Connection Struct

**File:** `src/rust/src/core/connection.rs`

In the `Connection<T>` struct definition (around line 357-410), add this field after the `bwe_callback_state` field:

```rust
pub struct Connection<T>
where
    T: Platform,
{
    // ... existing fields ...
    
    /// Tracks when to send `ConnectionObserverEvent::LowBandwidthForVideo`.
    bwe_callback_state: BweCallbackState,
    
    // Frame encryption context (must be outside of locks for WebRTC callbacks)
    frame_crypto_context: Arc<CallMutex<frame_crypto::Context>>,
}
```

## Step 3: Initialize frame_crypto_context in Connection::new()

**File:** `src/rust/src/core/connection.rs`

In the `Connection::new()` method (around line 540-620), add initialization after creating the `webrtc` field:

```rust
let connection = Self {
    // ... existing fields ...
    incoming_video_sink,
    bwe_callback_state: BweCallbackState::CheckIfLow {
        delayed_check_tick: 0,
    },
    
    // Initialize frame encryption context with random secret
    frame_crypto_context: Arc::new(CallMutex::new(
        frame_crypto::Context::new(frame_crypto::random_secret(&mut OsRng)),
        "frame_crypto_context",
    )),
};
```

## Step 4: Update Clone Implementation

**File:** `src/rust/src/core/connection.rs`

In the `impl Clone for Connection<T>` (around line 459-490), add the frame_crypto_context clone:

```rust
impl<T> Clone for Connection<T>
where
    T: Platform,
{
    fn clone(&self) -> Self {
        Connection {
            // ... existing clones ...
            incoming_video_sink: self.incoming_video_sink.clone(),
            bwe_callback_state: self.bwe_callback_state,
            frame_crypto_context: Arc::clone(&self.frame_crypto_context),  // <-- ADD THIS
        }
    }
}
```

## Step 5: Add Encryption Helper Methods to Connection

**File:** `src/rust/src/core/connection.rs`

Add these helper methods to `impl<T> Connection<T>` (find a good spot around line 492):

```rust
impl<T> Connection<T>
where
    T: Platform,
{
    // ... existing methods ...
    
    // Frame encryption buffer size calculations (from group_call.rs pattern)
    const FRAME_ENCRYPTION_FOOTER_LEN: usize = 
        std::mem::size_of::<frame_crypto::RatchetCounter>()
        + std::mem::size_of::<u32>()  // FrameCounter
        + std::mem::size_of::<frame_crypto::Mac>();

    pub fn get_ciphertext_buffer_size(plaintext_size: usize) -> usize {
        plaintext_size.saturating_add(Self::FRAME_ENCRYPTION_FOOTER_LEN)
    }

    pub fn get_plaintext_buffer_size(ciphertext_size: usize) -> usize {
        ciphertext_size.saturating_sub(Self::FRAME_ENCRYPTION_FOOTER_LEN)
    }

    fn encrypt_media(&self, plaintext: &[u8], ciphertext_buffer: &mut [u8]) -> Result<usize> {
        let mut frame_crypto_context = self
            .frame_crypto_context
            .lock()
            .expect("Get e2ee context to encrypt media");

        Self::encrypt_internal(&mut frame_crypto_context, plaintext, ciphertext_buffer)
    }

    fn decrypt_media(
        &self,
        remote_demux_id: DemuxId,
        ciphertext: &[u8],
        plaintext_buffer: &mut [u8],
    ) -> Result<usize> {
        let mut frame_crypto_context = self
            .frame_crypto_context
            .lock()
            .expect("Get e2ee context to decrypt media");

        Self::decrypt_internal(&mut frame_crypto_context, remote_demux_id, ciphertext, plaintext_buffer)
    }

    fn encrypt_internal(
        frame_crypto_context: &mut frame_crypto::Context,
        plaintext: &[u8],
        ciphertext_buffer: &mut [u8],
    ) -> Result<usize> {
        use crate::webrtc::sdp_observer::{Reader, Writer};
        
        let ciphertext_size = Self::get_ciphertext_buffer_size(plaintext.len());
        let mut ciphertext = Writer::new(ciphertext_buffer);

        let encrypted_payload = ciphertext.write_slice(plaintext)?;

        let mut mac = frame_crypto::Mac::default();
        let (ratchet_counter, frame_counter) =
            frame_crypto_context.encrypt(encrypted_payload, &mut mac)?;
        if frame_counter > u32::MAX as u64 {
            return Err(RingRtcError::FrameCounterTooBig.into());
        }

        ciphertext.write_u8(ratchet_counter)?;
        ciphertext.write_u32(frame_counter as u32)?;
        ciphertext.write_slice(&mac)?;

        Ok(ciphertext_size)
    }

    fn decrypt_internal(
        frame_crypto_context: &mut frame_crypto::Context,
        remote_demux_id: DemuxId,
        ciphertext: &[u8],
        plaintext_buffer: &mut [u8],
    ) -> Result<usize> {
        use crate::webrtc::sdp_observer::{Reader, Writer};
        
        let mut ciphertext_reader = Reader::new(ciphertext);
        let mut plaintext_writer = Writer::new(plaintext_buffer);

        let mac: frame_crypto::Mac = ciphertext_reader
            .read_slice_from_end(std::mem::size_of::<frame_crypto::Mac>())?
            .try_into()?;
        let frame_counter = ciphertext_reader.read_u32_from_end()?;
        let ratchet_counter = ciphertext_reader.read_u8_from_end()?;

        let encrypted_payload = plaintext_writer.write_slice_overlapping(ciphertext_reader.remaining())?;

        frame_crypto_context.decrypt(
            remote_demux_id,
            ratchet_counter,
            frame_counter as u64,
            encrypted_payload,
            &mac,
        )?;
        Ok(encrypted_payload.len())
    }
}
```

## Step 6: Implement Encryption Methods in PeerConnectionObserverTrait

**File:** `src/rust/src/core/connection.rs`

In the `impl<T> PeerConnectionObserverTrait for Connection<T>` (around line 2135-2190), add these methods after `handle_incoming_video_frame`:

```rust
impl<T> PeerConnectionObserverTrait for Connection<T>
where
    T: Platform,
{
    // ... existing methods ...

    fn handle_incoming_video_frame(
        &self,
        demux_id: DemuxId,
        _video_frame_metadata: VideoFrameMetadata,
        video_frame: Option<VideoFrame>,
    ) -> Result<()> {
        if let (Some(incoming_video_sink), Some(video_frame)) =
            (self.incoming_video_sink.as_ref(), video_frame)
        {
            incoming_video_sink.on_video_frame(demux_id, video_frame)
        }
        Ok(())
    }

    // Frame encryption support
    fn get_media_ciphertext_buffer_size(
        &mut self,
        _is_audio: bool,
        plaintext_size: usize,
    ) -> usize {
        Self::get_ciphertext_buffer_size(plaintext_size)
    }

    fn encrypt_media(&mut self, plaintext: &[u8], ciphertext_buffer: &mut [u8]) -> Result<usize> {
        self.encrypt_media(plaintext, ciphertext_buffer)
    }

    fn get_media_plaintext_buffer_size(
        &mut self,
        _track_id: u32,
        _is_audio: bool,
        ciphertext_size: usize,
    ) -> usize {
        Self::get_plaintext_buffer_size(ciphertext_size)
    }

    fn decrypt_media(
        &mut self,
        track_id: u32,
        ciphertext: &[u8],
        plaintext_buffer: &mut [u8],
    ) -> Result<usize> {
        let remote_demux_id = track_id;
        self.decrypt_media(remote_demux_id, ciphertext, plaintext_buffer)
    }
}
```

## Step 7: Enable Frame Encryption in call_manager

**File:** `src/rust/src/android/call_manager.rs`

In the `create_peer_connection()` function (around line 115-120), change:

```rust
// BEFORE:
let pc_observer = PeerConnectionObserver::new(
    connection_ptr,
    false, /* enable_frame_encryption */  // <-- CHANGE TO TRUE
    false, /* enable_video_frame_event */
    false, /* enable_video_frame_content */
)?;

// AFTER:
let pc_observer = PeerConnectionObserver::new(
    connection_ptr,
    true,  /* enable_frame_encryption */   // <-- CHANGED
    false, /* enable_video_frame_event */
    false, /* enable_video_frame_content */
)?;
```

## Summary of Changes

| File | Changes |
|------|---------|
| `connection.rs` | Add frame_crypto import, add field to struct, initialize in `new()`, update `Clone`, add helper methods, implement trait methods |
| `call_manager.rs` | Enable frame encryption flag in `create_peer_connection()` |

## Testing

1. Build the project: `cargo build`
2. Run 1:1 calls and verify media encryption is working
3. Check logs for encryption/decryption activity
4. Verify video and audio are transmitted correctly

## Reference Pattern

This implementation follows the same pattern as `group_call.rs`:
- Lines 906-910: frame_crypto_context field
- Lines 1263-1264: Context initialization
- Lines 3735-3850: encrypt/decrypt implementations
- Lines 5086-5127: PeerConnectionObserverImpl trait implementations
