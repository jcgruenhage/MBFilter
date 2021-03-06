use clap::{Arg, App, SubCommand};
use std::sync::Mutex;
use std::sync::Arc;
use moessbauer_filter::{
    MBConfig,
    MBFilter,
    MBFState,
};
use std::error::Error;
use std::fs::File;
use std::io::{
    BufWriter,
    Write,
};
use std::path::Path;
use mbfilter::MBError;
use log::{
    info,
    debug,
    error,
};
use warp::Filter;
use futures_util::stream::StreamExt;
use futures_util::FutureExt;
use futures_util::SinkExt;
use warp::http;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {

    // initiate logger
    env_logger::init();

    // parse the command line
    let matches = App::new("Moessbauer Filter")
        .version("0.1")
        .author("Alexander Becker <nabla.becker@mailbox.org>")
        .about("Program to interface with the Hardware on the FPGA")
        .subcommand(SubCommand::with_name("configure")
            .about("write a configuration to the filter. If the filter is currently running, the filter is halted,\
                the fifo emptied and then the filter is configured and placed in the ready state")
            .arg(Arg::with_name("k")
                .short("k")
                .long("k-param")
                .value_name("flank steepnes")
                .help("length of the rising and falling flank of the trapezoidal filter in filter clock cycles (8ns)")
                .takes_value(true)
                .required(true)
                .index(1))
            .arg(Arg::with_name("l")
                .short("l")
                .long("l-param")
                .value_name("plateau length")
                .help("length of the plateau of the trapezoidal filters in filter clock cycles")
                .takes_value(true)
                .required(true)
                .index(2))
            .arg(Arg::with_name("m")
                .short("m")
                .long("m-factor")
                .value_name("decay time factor")
                .help("multiplication factor of the filter. Sets the decay time that the filter is sensitive to")
                .takes_value(true)
                .required(true)
                .index(3))
            .arg(Arg::with_name("pthresh")
                .short("p")
                .long("pthresh")
                .value_name("peak threshhold")
                .help("minimum value of the peak to be considered as a signal")
                .takes_value(true)
                .required(true)
                .index(4))
            .arg(Arg::with_name("dead-time")
                .short("d")
                .long("dtime")
                .value_name("dead time")
                .help("the time in which the filter coalesses multiple peaks into a single peak for noise reduction")
                .takes_value(true)
                .required(true)
                .index(5)))
        .subcommand(SubCommand::with_name("server")
            .about("Turn the control program into a server that opens a specified port and waits for client connections")
            .arg(Arg::with_name("listen")
                .short("l")
                .long("listen")
                .value_name("listen")
                .help("the IP address and port that the server should listen on")
                .takes_value(true)
                .required(true)
                .index(1)))
        .subcommand(SubCommand::with_name("start")
            .about("command that starts the measurement. The filter has to be configured to be able to start")
            .arg(Arg::with_name("output file")
                .short("o")
                .long("ofile")
                .value_name("output file")
                .help("file path where the results of the measurement are written to CAUTION: Be aware of disk space")
                .takes_value(true)
                .index(1)
                .required(true))
            .arg(Arg::with_name("target file size")
                .short("s")
                .long("target-file-size")
                .help("The file size that should be collected before the measurement is automatically stopped")
                .takes_value(true)
                .required(true)
                .index(2)))
        .subcommand(SubCommand::with_name("status")
            .about("command that returns the current state of the hardware filter with the currently loaded configuration"))
        .subcommand(SubCommand::with_name("stop")
            .about("stops the filter if it is running"))
        .get_matches();

    // configure subcommand
    if let Some(matches) = matches.subcommand_matches("configure") {
        let filter = MBFilter::new()?;
        let config = MBConfig::new_from_str(
                    matches.value_of("k").unwrap(),
                    matches.value_of("l").unwrap(),
                    matches.value_of("m").unwrap(),
                    matches.value_of("pthresh").unwrap(),
                    matches.value_of("dead-time").unwrap())?;
        filter.configure(config);
        ()
    }

    // start subcommand
    if let Some(matches) = matches.subcommand_matches("start") {
        let mut filter = MBFilter::new()?;
        let requested_pc = u64::from_str_radix(matches.value_of("target file size").unwrap(), 10)?;
        let filepath = matches.value_of("output file").unwrap();
        let path = Path::new(filepath);
        let ofile = File::create(&path)?;
        let mut ofile = BufWriter::new(ofile);
        let mut fc: u64 = 0;
        match filter.state() {
            MBFState::Ready => {
                filter.start();
                let mut buffer: [u8; 12*2048] = [0; 12*2048];
                while fc < requested_pc {
                    let bytes_read = filter.read(&mut buffer)?;
                    debug!("{} bytes read", bytes_read);
                    let mut pos = 0;
                    while pos < (&buffer[..bytes_read]).len() {
                        let bytes_written = ofile.write(&buffer[pos..bytes_read])?;
                        pos += bytes_written;
                    };
                    fc += bytes_read as u64;
                }
                filter.stop();
            },
            _ => Err(MBError::WrongState)?,
        }
    }

    // server subcommand
    if let Some(matches) = matches.subcommand_matches("server") {
        let filter = Arc::new(Mutex::new(MBFilter::new()?));
        let socket_address: std::net::SocketAddr = matches.value_of("listen").unwrap().parse()?;
        let hello = warp::path("websocket")
            .and(warp::query::query())
            .and(warp::ws())
            .map(move |config, ws| {
                ws_handler(filter.clone(), config, ws);
                ""
            });
        warp::serve(hello)
            .run(socket_address)
            .await;
    }


    // stop subcommand
    if let Some(_) = matches.subcommand_matches("stop") {
        unimplemented!("stop subcommand")
    }

    // status subcommand
    if let Some(_) = matches.subcommand_matches("status") {
        if let Ok(filter) = MBFilter::new() {
            let config = filter.configuration();
            let state = filter.state();
            println!("{}\nCurrent filter State:\n{}", config, state);
        }
    }
    Ok(())
}

async fn read_task(filter: Arc<Mutex<MBFilter>>, ws: warp::ws::Ws) {
}


async fn ws_handler(filter: Arc<Mutex<MBFilter>>, config: MBConfig, ws: warp::ws::Ws) -> dyn warp::Reply {
    let config = match config.validate() {
        Ok(config) => config,
        Err(e) => panic!("AAAAH"),
    };
    let mut locked_filter = filter.try_lock();
    if let Ok(ref mut unlocked_filter) = locked_filter {
        match unlocked_filter.state() {
            MBFState::Ready | MBFState::InvalidParameters => {
                unlocked_filter.configure(config);
                //TODO: do websocket things
                //tokio::task::spawn(read_task(filter.clone(), ws));
            },
            _ => panic!("bbb"),//return warp::reply::with_status(format!("Filter already running"), http::status::StatusCode::TERMPORARILY_UNAVAILABLE),
        }
    }
}
