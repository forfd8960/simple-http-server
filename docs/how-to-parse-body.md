# How to parse HTTP request body correctly

本文总结了在 TCP 流上正确解析 HTTP/1.1 request body 的实用步骤，重点解决：

- 头和体跨多次 read 的问题
- 同一包内多请求（pipelining）的问题
- Content-Length 与已读字节的边界问题
- 大小限制与异常输入的防御

## 1. 总体思路

使用“连接级缓冲”（per-connection buffer）来解析：

1. 每条连接维护一个可增长缓冲区 conn_buf
2. 先解析 header，拿到 header_end（httparse 返回的 Complete(n)）
3. 根据分帧规则确定当前请求总长度 total_needed
4. 只消费当前请求所需字节
5. 剩余字节留在 conn_buf，给下一次请求解析

关键公式（仅 Content-Length 场景）：

- header_len = n
- total_needed = header_len + content_length

## 2. 详细步骤（Content-Length 版本）

### Step 1: 维护连接级缓冲

- 在每个连接处理循环外初始化 conn_buf
- 每次解析请求都复用这个 conn_buf

### Step 2: 循环读并解析 header

- 尝试对 conn_buf 做 httparse
- 若 Partial：
  - 若 conn_buf.len() > max_header_size，返回错误
  - 从 socket 继续 read 到 conn_buf
- 若 Complete(n)：得到 header_len = n

### Step 3: 读取并校验 Content-Length

- 大小写不敏感读取 header 名（建议统一存小写）
- 若有 Content-Length：
  - 解析为 usize
  - 非法值直接报错（400）
  - 若 > max_body_size，返回 413
- 若无 Content-Length：按 0 body 处理（本实现范围）

### Step 4: 确保缓冲区包含完整请求体

- 计算 total_needed = header_len + content_length
- 若 conn_buf.len() < total_needed，继续 read 直到达到
- 若 read 返回 EOF 且未满足 total_needed，报 UnexpectedEof

### Step 5: 精确消费当前请求

- body = conn_buf[header_len..total_needed]
- 将 body 赋给当前请求
- conn_buf.split_to(total_needed) 仅消费当前请求
- split 后 conn_buf 剩余字节可能是下一条请求，必须保留

### Step 6: 进入下一轮解析

- 下一轮直接从剩余 conn_buf 开始 parse，不需要丢弃

## 3. 常见错误与修复

### 错误 A: 用新建临时 buf 解析每个请求

问题：会吞掉后续请求字节，pipelining 失效。

修复：使用连接级 conn_buf，且只消费当前请求长度。

### 错误 B: 在 parse 前用 buf.len() 判断 header 超限

问题：可能把已预读的 body 算进 header，误判超限。

修复：在 Partial 阶段做 header_size 检查；Complete 后按 header_len 计算。

### 错误 C: content_length - already_read 直接相减

问题：若 already_read > content_length 可能下溢。

修复：使用 checked_sub 或直接比较并报错。

### 错误 D: Header 名大小写敏感

问题：漏掉 content-length 等变体。

修复：存储时统一小写，查找时用小写 key。

## 4. 伪代码

```text
conn_buf = BytesMut::new()

loop per connection:
  # parse request line + headers
  loop:
    parse_result = parse(conn_buf)
    if parse_result == Complete(header_len):
      req = build_request_from_headers(conn_buf[0..header_len])
      break

    if conn_buf.len() > max_header_size:
      return HeaderTooLarge

    n = read_into(conn_buf)
    if n == 0:
      return UnexpectedEof

  content_length = parse_content_length(req.headers) or 0
  if content_length > max_body_size:
    respond_413()
    discard_current_request_bytes(conn_buf, header_len, content_length)
    continue

  total_needed = header_len + content_length
  while conn_buf.len() < total_needed:
    n = read_into(conn_buf)
    if n == 0:
      return UnexpectedEof

  req.body = conn_buf[header_len..total_needed]
  split_to(conn_buf, total_needed) # consume only current request

  handle(req)
  write_response()
```

## 5. ASCII 流程图

```text
+-----------------------------+
| Start connection loop       |
+-------------+---------------+
              |
              v
+-----------------------------+
| Parse conn_buf with httparse|
+------+------+---------------+
       |Complete(n)   |Partial
       v              v
+----------------+  +------------------------------+
| header_len = n |  | if len > max_header_size err |
+-------+--------+  +---------------+--------------+
        |                           |
        |                           v
        |                 +------------------------+
        |                 | read more into conn_buf|
        |                 +-----------+------------+
        |                             |
        +-----------------------------+
                      (retry parse)

After Complete:

+------------------------------------------+
| parse Content-Length (default 0)         |
+------------------+-----------------------+
                   |
      > max_body_size ?
         |yes                    |no
         v                       v
+----------------------+   +------------------------------+
| respond 413 and      |   | total_needed = header_len+CL |
| discard current req  |   +--------------+---------------+
+----------+-----------+                  |
           |                              v
           |                 +------------------------------+
           |                 | read until len >= total_needed|
           |                 +--------------+---------------+
           |                                |
           |                                v
           |                 +------------------------------+
           |                 | body = [header_len..needed]  |
           |                 | split_to(total_needed)       |
           |                 +--------------+---------------+
           |                                |
           +--------------------------------+
                                            v
                                  +-------------------+
                                  | handle + response |
                                  +-------------------+
```

## 6. 当前实现边界（建议）

- 当前仅覆盖 Content-Length body
- 对 Transfer-Encoding: chunked 建议显式拒绝（400/501）或单独实现 chunk 解析
- 建议补充测试：
  - body 分片到达
  - 同包多请求 + 第一条含 body
  - Content-Length 非法/过大
  - EOF 提前中断
