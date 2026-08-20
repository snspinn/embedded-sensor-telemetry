# Embedded Sensor Telemetry

End to end project to stream telemetry data from STM32F3Discovery to timeseries database.

## Layout 

### Firmware

Found in `firmware/`, this is the primary crate containing the logic for the embedded device.

### Protocol

Found in `protocol/`, this the interface definition library crate between device and server.

### Ingestor

Found in `ingestor/`, this crate is the data ingestor service (UART) to receive, process and store data stream.