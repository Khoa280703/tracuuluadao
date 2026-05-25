Bạn nhận danh sách khoảng 30 kết quả tìm kiếm và phải lọc ra tối đa 10 URL điều tra sâu.

Quy tắc:
- Chọn các URL có xác suất cao là bài tố cáo/cảnh báo/scam report thật sự về đúng truy vấn đang tra.
- Mục tiêu là lấy ra 10 bài lừa đảo thật sự, không phải 10 URL bất kỳ.
- Ưu tiên bài viết cụ thể, group post, thread, bài cảnh báo, bài báo có case cụ thể, forum tố cáo, trang bóc phốt có nội dung trực tiếp.
- Không ưu tiên các domain đã có scraper riêng nếu vẫn còn đủ nguồn ngoài để điều tra. Chỉ chọn chúng khi chúng là bằng chứng mạnh hoặc thiếu nguồn tốt hơn.
- Ưu tiên URL bài viết cụ thể hơn trang chủ, tag, search page, landing page, profile tổng, page tổng hợp, bài hướng dẫn chung.
- Cố gắng giữ đa dạng nguồn và đa dạng case. Nếu nhiều URL mô tả cùng một bài/cùng một nội dung/cùng một screenshot thì chỉ giữ 1 URL mạnh nhất.
- Bỏ các URL rác: trang chủ, landing page, trang sản phẩm, tài liệu PDF/Office, file tải về, trang redirect, kết quả tìm kiếm, bài hướng dẫn chung, bài pháp luật chung, bài tin tức chung không phải case cụ thể.
- Nếu có nhiều URL gần như trùng nhau chỉ khác query param/tracking param hoặc mirror/cross-post cùng nội dung, chỉ giữ 1 URL tốt nhất.
- Chỉ được chọn URL đã xuất hiện nguyên văn trong đầu vào.
- Không chọn trang kết quả tìm kiếm, trang redirect hoặc trang trung gian của Google/DuckDuckGo.
- Ưu tiên các item có `flags.direct_scam_signal = true`.
- Tránh các item có `flags.generic_or_low_signal = true` trừ khi không còn lựa chọn nào khác.
- Đừng máy móc theo rank. Rank cao hữu ích nhưng không quan trọng bằng việc đúng là bài tố cáo/scam report.
- Không viết gì ngoài JSON.
