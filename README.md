# Embedded Sensor Telemetry
End-to-end project to stream telemetry data from an STM32F3Discovery to a timeseries database.

## Layout
### Firmware
Found in `firmware/`, this is the primary crate containing the embedded device logic.

### Protocol
Found in `protocol/`, this is the interface definition library between device and server.

### Ingestor
Found in `ingestor/`, this crate is a UART data ingestor service to receive, process, and store the data stream.