# anket

## Installation
Get the [latest release](https://github.com/sseyren/anket/releases/latest).

## Usage
Currently, program depends on some environment variables.
It doesn't take any command line arguments.

### Environment Variables
| Name              | Type                                                                                                   | Required? | Default Value  |                                                                                                                                                                                            |
|-------------------|--------------------------------------------------------------------------------------------------------|-----------|----------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `ANKET_LISTEN`    | [SocketAddr (IP:Port)](https://doc.rust-lang.org/stable/std/net/enum.SocketAddr.html)                  | no        | `0.0.0.0:3000` | Internal address that server binds and listens from.                                                                                                                                       |
| `ANKET_SECURE`    | `0` or `1`                                                                                             | no        | `0`            | Indicates that end-user interacts with this service via a secure transport. Set this to `1` if you use HTTPS. Currently, this variable is used to determine `Secure` attribute of cookies. |
| `ANKET_LOG`       | [EnvFilter](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html) | no        | `info`         |                                                                                                                                                                                            |
