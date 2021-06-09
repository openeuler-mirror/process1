# Process 1 设计思考

## 背景说明

在所有 Unix 系统中都有一个进程号为 1 的进程。这是操作系统内核完成启动后，执行的第一个用户态应用程序，所以进程号在数值上仅次于 idle 这个特殊进程。

从功能定位上看，init 所负责的最基本功能有两类：

1. 启停系统服务: 比如 init 负责在启动阶段拉起用户空间的各个守护进程，这个过程需要依赖文件系统挂载、设备发现、网络配置等活动。相应的，在系统关闭或者重启过程中，init 同样要负责关闭网络接口，数据落盘，资源清理，卸载文件系统等等。系统关闭过程的不完善，会影响下一次能否成功启动。
2. 回收孤儿进程: 在 Unix 系统上，如果父进程比子进程更早退出，子进程成为孤儿的情况下，init 有回收孤儿进程运行状态的责任。

从用户的角度看，回收孤儿进程是个基本功能，这点如果没做到，系统会出现大量僵尸进程，影响系统稳定性。但更重要的还是对系统服务启停的管理。一个优秀的init进程，应该具备快速、可靠的特性。为达成快速的目的，有几个关键的手段：

- 按需启动：启动时，只启动必须的服务，将其他事务延后到合适的时机（真正需要它的时候），比如更新、上报等服务等到系统空闲的时候再处理。
- 更多的并行：保持足够高的并发度，最大化的利用CPU、内存、带宽资源，使系统快速行进至登陆界面，而不是白白浪费时间。
- 保证每个启动脚本的高性能：避免不必要的fork和脚本。

## 主流init实现方式对比

很多人一直努力地从某些方面改进传统的 init 守护进程，使它变得更完善。有简洁可靠但低效的sysvinit，有高效但略显复杂的systemd。

*todo* 增加对 Android init 的分析对比

| Init软件 | 说明                                                         | 启动管理 | 进程回收 | 服务管理 | 并行启动 | 设备管理 | 资源控制 | 日志管理 |
| -------- | ------------------------------------------------------------ | -------- | -------- | -------- | -------- | -------- | -------- | -------- |
| sysvinit | 早期版本使用的初始化进程工具,  逐渐淡出舞台。                | ✓        | ✓        |          |          |          |          |          |
| upstart  | debian,  Ubuntu等系统使用的initdaemon                        | ✓        | ✓        | ✓        | ✓        |          |          |          |
| systemd  | 提高系统的启动速度，相比传统的System  V是一大革新，已被大多数Linux发行版所使用。 | ✓        | ✓        | ✓        | ✓        | ✓        | ✓        | ✓        |

## systemd 的问题分析

systemd 目前已成为大多数发行版的标准配置。
systemd是为了解决启动时间长，启动脚本复杂问题而诞生的。它的设计目标是，为系统的启动和管理提供一整套完整的解决方案。

从 openEuler 集成 systemd 的实践结果看，我们认为 systemd 虽然对用户使用感受比较友好，但对开发者来说，存在很多待改进的问题：

1. 版本迭代快，每月近千提交，不提供成熟稳定的社区版本；
2. C语言编写，代码质量不高，提交中常有低级编程错误，bug多；
3. 社区的维护策略是强制大家使用最新的不稳定版本，maintainer 经常用 version too old 来应付求助和讨论；
以上三点使得 systemd 的维护成本非常高；

4. 系统韧性不强。模块耦合严重，问题容易在模块间扩散，sd-event、sd-bus、unit等机制缠绕在一起，一旦出问题，就导致系统不可用或崩溃；
5. 集成100+的特性，并且与其它软件包功能有重合，在复杂场景中使用时，集成测试难度非常高；
6. 调试检测手段匮乏，经常满屏幕打印assert，反而容易造成关键信息被淹没；
7. 边界不清晰，部分功能和其他软件包重合，一旦出现问题，需要很多不同背景的人共同分析定位。对一些系统能力管的宽但管的不彻底，比如cgroup就是一个例证，配置项竟然另起一套；
以上四点使得 systemd 在新增特性新增场景时格外容易出错，而一旦出错就影响系统整体的可靠性；

8. 向下兼容的工作量大，对于刚从传统 sysvinit 转过来的服务，有些行为显得莫名其妙；
9. 新人培养的门槛高，机制复杂，远不如 sysv init 那么简单直观；
以上两点使得我们的 legacy 业务向新系统的迁移变得没有必要的复杂。

总的来说，我们认为 systemd 的架构和社区运作定位，使得它不适应 云场景中业务快速开发迭代的诉求。云原生场景下，需要一个能够在 业务快速开发迭代中不使绊子、不拖后腿的 init 系统。

## 设计的目标和约束

**设计目标**

- **皮实耐操**: 一个未充分测试的新特性，只会影响到他自身的稳定性，不会影响系统整体的稳定性。
- **管理便捷**：可以方便的安装/部署/升级/运维
- **足够轻量**：占用更少的资源，systemd 大约需要 100M 以上的安装空间。我们新的设计实现能够控制在 个位数。
- **使用方便**：更灵活简洁的架构，易于理解、便于运维
- **极致安全**：保持对进程的跟踪，提供最小的运行环境

## 设计思路

### 简洁但不简单，打造轻量级底座(功能裁剪/自研替代)

**趋势**

- 各大OS发行商均发布轻量级OS
- 1号进程越来越厚重,打造"用户态kernel"
- 传统OS越来越不适应云边端

**思路**

•提供process1底座， **框架+组件**的形式，将纷繁的关系简化，封装在模块内部, 方便选配、管理、升级

•为init进程减负，只提供框架，做最少的事，减轻故障时影响

•**集成或自研**必要的库和工具，保证OS足够轻量，只提供必要的基础的功能、工具，其他容器化、虚拟机化

•**提供统一、轻量的底座，**kernel+process1, **即最小OS**

### 组件化定制/版本控制，适应云、边、端等场景

**思路**

•提供统一的框架，力争**屏蔽底层差异**

•**组件化**，可根据硬件**自由选配**

•组件**版本控制**，轻松应对安装/部署/升级

### 更加可靠，保活+自恢复

**现状**

•1号进程承担的功能太多，触发问题概率高

•代码质量不高，安全性差

**思路**

•解放1号进程，只做必要的事情，启动+进程回收+关键模块保活，**永远在线**

•1号进程故障自恢复，组件保活

•使用RUST，安全、并发、高效、实用

•故障监测/上报

## 分层视图

## 架构图

## 代码目录结构说明