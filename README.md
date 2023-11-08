# anket

## Installation
Get the [latest release](https://github.com/sseyren/anket/releases/latest).

## Usage
Currently, program depends on some environment variables.
It doesn't take any command line arguments.

### Environment Variables
| Name              | Type                                                                                                   | Required? | Default Value  |                                                                                                                                                                                                                                                                  |
|-------------------|--------------------------------------------------------------------------------------------------------|-----------|----------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `ANKET_ROOT`      | [URI](https://docs.rs/http/latest/http/uri/struct.Uri.html)                                            | yes       |                | This determines address of this instance for end-user. This is mostly used from client side of the project. (Examples: `http://127.0.0.1:3000`, `https://anket.mywebsite.com`)                                                                                   |
| `ANKET_LISTEN`    | [SocketAddr (IP:Port)](https://doc.rust-lang.org/stable/std/net/enum.SocketAddr.html)                  | no        | `0.0.0.0:3000` | Internal address that server binds and listens from.                                                                                                                                                                                                             |
| `ANKET_IP_LOOKUP` | `0` or `1`                                                                                             | no        | `0`            | Normally, user identification done by using a session cookie. This option takes IP address of user into consideration. It takes it from leftmost address of `X-Forwarded-For` header of request, if it's set. If it's not, it uses incoming IP address directly. |
| `ANKET_LOG`       | [EnvFilter](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html) | no        | `info`         |                                                                                                                                                                                                                                                                  |
