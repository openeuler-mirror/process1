//! socket_config模块socket类型配置文件的定义，以及保存配置文件解析之后的内容
//!
#![allow(non_snake_case)]
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use confique::{Config, Error};
use process1::manager::DeserializeWith;
use process1::manager::ExecCommand;

use crate::socket_base::SocketCommand;

pub(super) enum ListeningItem {
    Stream,
    Datagram,
    Netlink,
}

pub struct SocketConfig {
    data: Rc<RefCell<SocketConfigData>>,
}

impl SocketConfig {
    pub(super) fn new() -> Self {
        SocketConfig {
            data: Rc::new(RefCell::new(SocketConfigData::default())),
        }
    }

    pub(super) fn load(&self, paths: &Vec<PathBuf>) -> Result<(), Error> {
        let mut builder = SocketConfigData::builder().env();

        // fragment
        for v in paths {
            builder = builder.file(&v);
        }

        *self.data.borrow_mut() = builder.load()?;
        Ok(())
    }

    pub(super) fn config_data(&self) -> Rc<RefCell<SocketConfigData>> {
        self.data.clone()
    }

    pub(super) fn get_exec_cmds(&self, cmd_type: SocketCommand) -> Option<Vec<ExecCommand>> {
        self.data.borrow().get_exec_cmds(cmd_type)
    }
}

#[derive(Config, Default, Debug)]
pub(crate) struct SocketConfigData {
    #[config(nested)]
    pub Socket: SectionSocket,
}

#[derive(Config, Default, Debug)]
pub(crate) struct SectionSocket {
    #[config(deserialize_with = Vec::<ExecCommand>::deserialize_with)]
    pub ExecStartPre: Option<Vec<ExecCommand>>,
    #[config(deserialize_with = Vec::<ExecCommand>::deserialize_with)]
    pub ExecStartChown: Option<Vec<ExecCommand>>,
    #[config(deserialize_with = Vec::<ExecCommand>::deserialize_with)]
    pub ExecStartPost: Option<Vec<ExecCommand>>,
    #[config(deserialize_with = Vec::<ExecCommand>::deserialize_with)]
    pub ExecStopPre: Option<Vec<ExecCommand>>,
    #[config(deserialize_with = Vec::<ExecCommand>::deserialize_with)]
    pub ExecStopPost: Option<Vec<ExecCommand>>,
    pub ListenStream: Option<String>,
    pub ListenDatagram: Option<String>,
    pub ListenNetlink: Option<String>,
    pub PassPacketInfo: Option<bool>,
    pub Accept: Option<bool>,
    pub Service: Option<String>,
    pub ReceiveBuffer: Option<u64>,
    pub SendBuffer: Option<u64>,
    pub PassCredentials: Option<bool>,
    #[config(deserialize_with = Vec::<String>::deserialize_with)]
    pub Symlinks: Option<Vec<String>>,
    pub PassSecurity: Option<bool>,
    pub SocketMode: Option<u32>,
}

impl SocketConfigData {
    pub(super) fn get_exec_cmds(&self, cmd_type: SocketCommand) -> Option<Vec<ExecCommand>> {
        match cmd_type {
            SocketCommand::StartPre => self.Socket.ExecStartPre.clone(),
            SocketCommand::StartChown => self.Socket.ExecStartChown.clone(),
            SocketCommand::StartPost => self.Socket.ExecStartPost.clone(),
            SocketCommand::StopPre => self.Socket.ExecStopPre.clone(),
            SocketCommand::StopPost => self.Socket.ExecStopPost.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::socket_config::SocketConfig;
    use std::{
        env,
        ffi::OsString,
        fs::read_dir,
        io::{self, ErrorKind},
        path::PathBuf,
    };

    #[test]
    fn test_socket_parse() {
        let mut file_path = get_project_root().unwrap();
        file_path.push("libutils/examples/test.socket.toml");
        let mut paths = Vec::new();
        paths.push(file_path);

        let config = SocketConfig::new();
        let result = config.load(&paths);

        assert_eq!(result.is_err(), false);
    }

    fn get_project_root() -> io::Result<PathBuf> {
        let path = env::current_dir()?;
        let mut path_ancestors = path.as_path().ancestors();

        while let Some(p) = path_ancestors.next() {
            let has_cargo = read_dir(p)?
                .into_iter()
                .any(|p| p.unwrap().file_name() == OsString::from("Cargo.lock"));
            if has_cargo {
                return Ok(PathBuf::from(p));
            }
        }
        Err(io::Error::new(
            ErrorKind::NotFound,
            "Ran out of places to find Cargo.toml",
        ))
    }
}
