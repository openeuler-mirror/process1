1. 下载[systemd-248-13.oe1.src.rpm](https://repo.openeuler.org/openEuler-21.09/source/Packages/systemd-248-13.oe1.src.rpm)
2. 执行`rpm -ivh systemd-248-13.oe1.src.rpm`, 源码安装到`~/rpmbuild`
3. 将patches目录下的内容拷贝覆盖到`~/rpmbuild/SOURCES`
4. 执行`rpmbuild -ba ~/rpmbuild/SOURCES/systemd.spec --nocheck`
5. 生成的rpm在`~/rpmbuild/RPMS/x86_64/`下