use std::fs::File;
use std::io::{self, BufRead};
use std::path::Path;

pub struct FSTabItem {
    pub device_spec: String,
    pub mount_point: String,
    pub fs_type: String,
    pub options: String,
    pub dump: i8,
    pub pass: i8,
    pub state: i8,
}

impl FSTabItem {
    pub fn new(input: Vec<&str>) -> Self {
        let mut real_path = String::from(input[0]);
        if real_path.starts_with("UUID") {
            let uuid = String::from(&real_path["UUID".len() + 1..]);
            real_path = String::from("/dev/disk/by-uuid/") + &uuid;
        }
        FSTabItem {
            device_spec: String::from(real_path),
            mount_point: String::from(input[1]),
            fs_type: String::from(input[2]),
            options: String::from(input[3]),
            dump: String::from(input[4]).parse().unwrap(),
            pass: String::from(input[5]).parse().unwrap(),
            state: 0,
        }
    }
}

fn read_lines<P>(filename: P) -> io::Result<io::Lines<io::BufReader<File>>>
where
    P: AsRef<Path>,
{
    let file = File::open(filename)?;
    Ok(io::BufReader::new(file).lines())
}

pub fn parse(filename: &str) -> Vec<FSTabItem> {
    let mut res: Vec<FSTabItem> = Vec::new();
    if let Ok(lines) = read_lines(filename) {
        for line in lines {
            if let Ok(item_raw) = line {
                let item = item_raw.trim();
                if item.starts_with("#") || item.len() == 0 {
                    continue;
                }
                let mount: Vec<&str> = item.split_whitespace().collect();
                let fstab_item = FSTabItem::new(mount);
                res.push(fstab_item);
            }
        }
    } else {
        log::error!("Failed to open {}", filename);
    }
    res
}

#[cfg(test)]
mod tests {
    use std::fs::{remove_file, File};
    use std::io::prelude::*;
    use std::path::Path;

    use super::parse;

    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }

    #[test]
    fn test_parse() {
        let contents = "#
        # /etc/fstab
        # Created by anaconda on Sat Jul  9 16:27:31 2022
        #
        # Accessible filesystems, by reference, are maintained under '/dev/disk/'.
        # See man pages fstab(5), findfs(8), mount(8) and/or blkid(8) for more info.
        #
        # After editing this file, run 'systemctl daemon-reload' to update systemd
        # units generated from this file.
        #
        /dev/mapper/openeuler_192-root /                       ext4    defaults        1 1
        UUID=452b7bd2-c3ba-45d6-ab69-5d10d5140249 /boot                   ext4    defaults        1 2
        /dev/mapper/openeuler_192-swap none                    swap    defaults        0 0
        ";
        let path = Path::new("./fstab");

        let mut file = match File::create(&path) {
            Err(why) => panic!("couldn't create {:?}: {:?}", path, why),
            Ok(file) => file,
        };
        match file.write_all(contents.as_bytes()) {
            Err(why) => {
                panic!("couldn't write to {:?}: {:?}", path, why)
            }
            Ok(_) => println!("successfully wrote to {:?}", path),
        }

        let fstab_items = parse("./fstab");

        assert_eq!(fstab_items.len(), 3);

        assert_eq!(fstab_items[0].device_spec, "/dev/mapper/openeuler_192-root");
        assert_eq!(fstab_items[0].mount_point, "/");
        assert_eq!(fstab_items[0].fs_type, "ext4");
        assert_eq!(fstab_items[0].options, "defaults");

        assert_eq!(
            fstab_items[1].device_spec,
            "/dev/disk/by-uuid/452b7bd2-c3ba-45d6-ab69-5d10d5140249"
        );
        assert_eq!(fstab_items[1].mount_point, "/boot");
        assert_eq!(fstab_items[1].fs_type, "ext4");
        assert_eq!(fstab_items[1].options, "defaults");

        assert_eq!(fstab_items[2].device_spec, "/dev/mapper/openeuler_192-swap");
        assert_eq!(fstab_items[2].mount_point, "none");
        assert_eq!(fstab_items[2].fs_type, "swap");
        assert_eq!(fstab_items[2].options, "defaults");

        if path.exists() {
            match remove_file(path) {
                Ok(_) => {}
                Err(_) => {
                    println!("Failed to remove ./fstab");
                }
            }
        }
    }
}
