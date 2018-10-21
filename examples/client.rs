// Copyright (c) 2018, Alessandro Ghedini
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are
// met:
//
//     * Redistributions of source code must retain the above copyright
//       notice, this list of conditions and the following disclaimer.
//
//     * Redistributions in binary form must reproduce the above copyright
//       notice, this list of conditions and the following disclaimer in the
//       documentation and/or other materials provided with the distribution.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS
// IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO,
// THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR
// PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR
// CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
// EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
// PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
// PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
// LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
// NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
// SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

#[macro_use]
extern crate log;
extern crate docopt;
extern crate quiche;
extern crate env_logger;

use std::net;

use docopt::Docopt;

use quiche::rand;

const LOCAL_CONN_ID_LEN: usize = 16;

const TRANSPORT_PARAMS: quiche::TransportParams = quiche::TransportParams {
    idle_timeout: 30,
    initial_max_data: 10000000,
    initial_max_bidi_streams: 100,
    initial_max_uni_streams: 100,
    max_packet_size: 1500,
    ack_delay_exponent: 3,
    disable_migration: true,
    max_ack_delay: 25,
    initial_max_stream_data_bidi_local: 1000000,
    initial_max_stream_data_bidi_remote: 1000000,
    initial_max_stream_data_uni: 1000000,
    stateless_reset_token_present: true,
    stateless_reset_token: [0xba; 16],
};

const USAGE: &'static str = "Usage: client [options]

Options:
  -h --help          Show this screen.
  --connect <addr>   Connect to the given IP:port [default: 127.0.0.1:4433]
";

fn main() {
    let mut buf = [0; TRANSPORT_PARAMS.max_packet_size as usize];
    let mut out = [0; TRANSPORT_PARAMS.max_packet_size as usize];

    env_logger::init();

    let args = Docopt::new(USAGE)
                      .and_then(|dopt| dopt.parse())
                      .unwrap_or_else(|e| e.exit());

    let socket = net::UdpSocket::bind("0.0.0.0:0").unwrap();
    socket.connect(args.get_str("--connect")).unwrap();

    let mut scid: [u8; LOCAL_CONN_ID_LEN] = [0; LOCAL_CONN_ID_LEN];
    rand::rand_bytes(&mut scid[..]);

    let config = quiche::Config {
        version: quiche::VERSION_DRAFT15,

        local_conn_id: &scid,

        local_transport_params: &TRANSPORT_PARAMS,

        tls_server_name: "",
        tls_certificate: "",
        tls_certificate_key: "",
    };

    let mut conn = quiche::Conn::new(config, false).unwrap();

    let write = match conn.send(&mut out) {
        Ok(v) => v,

        Err(e) => panic!("{} initial send failed: {:?}", conn.trace_id(), e),
    };

    socket.send(&out[..write]).unwrap();

    debug!("{} written {}", conn.trace_id(), write);

    let mut req_sent = false;

    loop {
        let len = socket.recv(&mut buf).unwrap();
        debug!("{} got {} bytes", conn.trace_id(), len);

        let buf = &mut buf[..len];

        let mut left = len;

        // Process potentially coalesced packets.
        while left > 0 {
            let read = match conn.recv(&mut buf[len - left..len]) {
                Ok(v)  => v,
                Err(e) => panic!("{} recv failed: {:?}", conn.trace_id(), e),
            };

            left -= read;
        }

        if conn.is_established() && !req_sent {
            info!("{} sending HTTP request", conn.trace_id());

            let req = b"GET /index.html\r\n";
            conn.stream_send(4, &req[..], true).unwrap();

            req_sent = true;
        }

        let streams: Vec<u64> = conn.stream_iter().collect();
        for s in streams {
            let data = conn.stream_recv(s).unwrap();

            info!("{} stream {} has {} bytes (fin? {})",
                  conn.trace_id(), s, data.len(), data.fin());
        }

        loop {
            let write = match conn.send(&mut out) {
                Ok(v) => v,

                Err(quiche::Error::NothingToDo) => {
                    debug!("{} done writing", conn.trace_id());
                    break;
                },

                Err(e) => panic!("{} send failed: {:?}", conn.trace_id(), e),
            };

            // TODO: coalesce packets.
            socket.send(&out[..write]).unwrap();

            debug!("{} written {}", conn.trace_id(), write);
        }
    }
}
