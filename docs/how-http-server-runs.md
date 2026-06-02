# How HTTP Server Runs (Step-by-step mapping to server.rs)

本文是对当前实现的逐步映射，目标是回答两件事：

1. 请求从哪里进入
2. body 按什么规则被正确读取并只消费当前请求字节

对应源码文件：src/server.rs

---

## 0. 启动与监听

Step:
- 启动服务器，绑定地址，循环 accept 新连接。

Function:
- Server::new
- Server::serve

Key lines:
- Server 配置结构：117-120
- new：123-125
- bind + accept + spawn：127-147

说明：
- 每条连接会启动一个独立任务，进入 handle_stream。

---

## 1. 连接级循环入口（核心）

Step:
- 每条连接初始化一个 conn_buf，并在循环中持续解析请求。

Function:
- handle_stream

Key lines:
- 函数入口：151
- 连接级缓冲初始化：152
- 单连接循环：154-189

说明：
- conn_buf 是“连接级缓冲”，不是“单请求临时缓冲”。
- 这是支持 pipelining 的关键：多读到的字节不会丢。

---

## 2. 先解析请求头（不急着读完整 body）

Step:
- 调用 read_request，从 conn_buf 中尝试 parse。
- 成功时返回 (req, header_len)。

Function:
- read_request

Key lines:
- 调用点（handle_stream）：155
- read_request 定义：192-223
- parse Complete 返回 header_len：203-207

说明：
- header_len 即 httparse 的 Complete(n) 里的 n。
- 这个 n 是 body 起始偏移，也是后续计算 total_needed 的基础。

---

## 3. Header 不完整时继续读，直到能 parse Complete

Step:
- parse 结果为 Partial 时，检查头大小限制，然后继续读到 conn_buf。

Function:
- read_request

Key lines:
- Partial 分支：208-220
- max_header_size 检查：209-213
- read_buf 继续读取：215
- EOF 防御：216-217

说明：
- 这里的大小限制是“头阶段”保护，避免无上限增长。

---

## 4. 标准化 header key，便于大小写无关读取

Step:
- 从 httparse 转换到 HttpRequest 时，把 header 名统一转小写。

Function:
- HttpRequest::from_http_parse

Key lines:
- 定义：82-95
- to_lowercase 插入 headers：85-87

说明：
- 后续读取 Content-Length 使用 content-length 小写键。

---

## 5. 解析 Content-Length 并做 body 上限校验

Step:
- 在 handle_stream 里读取 content-length。
- 转换失败时报 ParseHeaderValueFailed。
- 超过 max_body_size 时返回 413。

Function:
- handle_stream
- HttpResponse::new_entity_too_large

Key lines:
- 读取 content-length：158
- parse usize：159-167
- body 上限判断：168
- 413 响应：169
- 413 构造函数：37-43

说明：
- 这是对大 body 的第一层保护。

---

## 6. 413 后要把“当前请求体”正确丢弃，避免协议错位

Step:
- 发送 413 后调用 discard_current_request_body 清理当前请求剩余 body。

Function:
- discard_current_request_body

Key lines:
- 调用点：170-171
- 函数定义：242-272
- 直接 split_to(total_needed)：251-254
- 不足则继续 read 丢弃：257-269

说明：
- 这一步保证连接可以继续处理后续请求，不会把上一条超限请求的残留字节误当作下一条请求。

---

## 7. 计算当前请求需要的总字节并补齐缓冲

Step:
- total_needed = header_len + content_length。
- 如果 conn_buf 不够长，循环读取直到达到 total_needed。

Function:
- handle_stream
- ensure_buffer_len

Key lines:
- 计算 total_needed：175
- 补齐调用：176
- ensure_buffer_len 定义：225-240
- 循环读到 target_len：233-238

说明：
- 这是“按 Content-Length 读满 body”的关键。

---

## 8. 只消费当前请求字节，保留后续字节

Step:
- body = conn_buf[header_len..total_needed]
- conn_buf.split_to(total_needed)

Function:
- handle_stream

Key lines:
- set_body：178
- split_to(total_needed)：179

说明：
- 若 conn_buf 中还有字节，它们是下一条请求的候选数据，必须保留。
- 这正是 pipelining 正常工作的核心行为。

---

## 9. 没有 Content-Length 时按空 body 处理并只消费头

Step:
- req.set_body([])
- conn_buf.split_to(header_len)

Function:
- handle_stream

Key lines:
- 无 CL 分支：182-185

说明：
- 这条路径适用于本实现中“无 body”请求。

---

## 10. 写回响应

Step:
- 构造 HttpResponse，并通过 write_response 输出。

Function:
- HttpResponse::new
- write_response

Key lines:
- 构造 200 响应：187
- write_response 调用：188
- write_response 定义：274-299

---

## 11. 回归测试如何验证连接级缓冲行为

Step:
- 单请求解析
- 同包两请求
- 第一条含 Content-Length body + 第二条紧跟

Function:
- tests::test_read_request
- tests::test_read_request_pipelined_two_requests_in_one_packet
- tests::test_read_request_pipelined_first_has_content_length_body

Key lines:
- 测试模块：301-387
- 单请求：307-326
- 同包两请求：328-350
- 第一条含 body + 第二条：352-386

---

## 12. 端到端时序（ASCII）

```text
[serve accept] -> [spawn handle_stream]
                    |
                    v
             init conn_buf once
                    |
                    v
              read_request parse header
               | Complete(header_len)
               | Partial -> keep reading into conn_buf
                    |
                    v
            if content-length exists?
                | yes                     | no
                v                         v
      parse + validate CL            body = empty
                |                         |
      > max_body_size ?                   |
         | yes         | no               |
         v             v                  |
      respond 413   ensure_buffer_len     |
      discard body  to header_len + CL    |
         |             |                  |
         +-------------+------------------+
                       v
            set req.body from conn_buf
            split_to(current request bytes)
            (leave extra bytes for next request)
                       |
                       v
                 write_response
                       |
                       v
                 next loop round
```

---

## 13. 一句话总结

当前实现通过 conn_buf + header_len + total_needed + split_to 的组合，确保了：

1. body 按 Content-Length 读取完整
2. 只消费当前请求字节
3. 剩余字节保留给下一次解析（支持 pipelining）
4. 对 header/body 大小和 EOF 有基本防御
