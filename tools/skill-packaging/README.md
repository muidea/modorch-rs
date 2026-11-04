# Skill Bundle 本地构建与安装

适用：modorch 0.0.2 开发树、Skill revision 1.0.5 / Contract revision 1.1.0；Python >= 3.11、Rust >= 1.85、锁定依赖。只生成本地 Candidate 工件，发布另受 release-gate 约束。

## 刷新与构建

源码/Rustdoc、Contracts、指南和模板是受控来源；references/assets/scripts 中的快照由 sync.py 生成，不手工编辑。修改公共内容时升级 Skill revision 并重新验证兼容性。开发树只声明已验证的 0.0.2 及其精确源码指纹，不泛化为所有 0.x。

```bash
python3 tools/skill-packaging/sync.py
python3 tools/skill-packaging/sync.py --check
python3 tools/doc-validation/run.py
python3 tools/skill-packaging/bundle.py build --output /tmp/modorch-skills-1.0.5.tar.gz
python3 tools/skill-packaging/bundle.py verify /tmp/modorch-skills-1.0.5.tar.gz
python3 tools/skill-packaging/bundle.py install /tmp/modorch-skills-1.0.5.tar.gz --dest /tmp/my-app/.agents/skills
```

输出归档包含 manifest.json、MIT LICENSE、README 和三个完整 Skill。安装需显式给目标路径；现有相同内容幂等，冲突拒绝，不覆盖、不写全局目录、不修改 Cargo。安装器只接受常规文件，拒绝穿越、绝对路径、链接、重复成员、超出预算和摘要不符。哈希验证完整性，来源可信性仍由用户核对版本与工件来源。

## 哈希与版本规则

逐文件 SHA-256 对原始字节计算。目录树摘要按 UTF-8 路径字典序排列，逐行拼接 `path + NUL + lowercase_sha256 + LF`，再对其 UTF-8 字节做 SHA-256。Bundle manifest 的 files 覆盖 LICENSE / README / skills 全部文件，不含自引用 manifest；归档 SHA-256 由 build 输出。gzip 时间戳与 Tar 权限（脚本 755、其他文件 644）、Owner、时间固定，可重复构建相同字节。

框架指纹按 Cargo metadata 中八个框架 package 的 `package-name/Cargo.toml` 与 `package-name/src/**/*.rs` 原始字节计算；应用自身与构建产物不参与。Feature 子集仅比较实际解析到的框架包，并拒绝未知 Feature / 缺少 runtime。每个 Skill 的 sources.json 记录参考来源摘要与渲染后摘要。workspace 版本继承由解析出的 package.version 另行核对。

## 外部使用与离线验证

添加 Cargo 依赖不会安装 Skill。安装后先运行指定 Skill 的 scripts/preflight.py --manifest-path <应用>/Cargo.toml。它离线读取真实依赖图并校验指纹；缺 lock/缓存/工具或不匹配返回非零及 Blocked，测试未执行明确 NotRun。版本字符串相同不能绕过源码匹配。

scripts/init-template.py 将 assets 中 Cargo.toml.in 渲染到新目录，路径参数作为 TOML 字符串编码，同时携带模板 Cargo.lock；随后由宿主策略获取依赖并检查锁文件，再运行 scripts/validate.sh。脚本不主动联网或安装工具。外部报告可用 scripts/validate-report.py 校验 references/result.schema.json。

## 宿主兼容

本体采用 Agent Skills 开放格式，.agents/skills 为仓库支持路径。Codex、Claude Code、Gemini CLI、Workorch 的自动发现、目录和调用方式需在具体客户端版本实际确认。当前提供[手工载入适配说明](../../integrations/README.md)，不声称所有宿主自动发现或跨模型评估已通过。

## 验证与发布

执行 Python tooling tests、独立应用模板 smoke、篡改/穿越/冲突/幂等与版本不匹配测试。实际 Agent 任务与 fixture replay 分开记录，入口见 tests/agent-evals/README.md。release-gate 缺任一 P0 证据返回非零，Candidate 包不能因文件完整而称 Release Ready。当前没有自动上传、系统安装或远程发布步骤。
