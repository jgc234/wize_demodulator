# WIZE Meter RTL-SDR Demodulator

Using and RTL-SDR device, this program can extract extract Wize protocol frames from all channels simultaneously.

Wize is a LP-WAN protocol used for devices like water meters. It is based on EN-13757 and similar to Wireless MBus Mode-N on 169.4MHz.

This module only decodes GFSK transmissions at 2400 baud.  It can take input from RTL-TCP or from a file written with RTL-SDR (IQ 8-bit unsigned complex pairs)

In summary:

- takes input from a file or rtl-tcp
- filters out a narrow band for each channel - ignores channel 1.
- decimates (downsamples) the signal in the filtering process.
- does a sliding-window style match on the bitstream to find the preamble and sync
- extracts the frame and does a quick CRC check.
- outputs to MQTT or stdout.

It does not decode the frame into layers (MBus, Wize, Application).  Be aware that the actual customer
meter data is encrypted and wont be readable, but there's many other interesting pieces of data that are 
not encrypted.

## History

This stemmed from a personal project to try and detect transmissions from a local rollout of smart water meters
by our local water company and understand the protocol being used.  It started with a GNURadio setup that 
eventually worked well - again using RTL-SDR as an input.  The installation and in-tree modules for GNURadio was 
fairly convoluted.  This version was an attempt to learn how to write Rust and experiment with FIR filters, and 
create a self-contained binary.  It does not have the sophistication or flexibility of GNURadio.

## Build

Using rust,

```bash
cargo build --release
```

Or build the container and use that instead.

### Run

You will need an RTL-SDR dongle, and RTL-TCP configured first.  That sample rate I use is 1.536M samples/second.

```bash
./target/release/rtl_pipeline --addr=10.0.0.23:1234 --gain=23 --ppm=-4 --mqtt-server=mqtt.example.org --mqtt-topic=wmbus/pdu
```
