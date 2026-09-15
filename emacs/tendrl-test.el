;;; tendrl-test.el --- live checks for tendrl.el  -*- lexical-binding: t; -*-

;; Needs an engine on `tendrl-engine-url' holding the example kasten
;; (30040:dc4cd086…:example) and `nak' on PATH.  Run:
;;
;;   emacs -Q --batch -L emacs -l tendrl -l nostrdown-mode -l emacs/tendrl-test.el
;;
;; Exits non-zero on any FAIL.  The engine relay fetch runs only in Auto
;; network mode (Confirm mode would wait for a click in the tendrl UI).

;;; Code:

(require 'tendrl)
(require 'nostrdown-mode)

(defvar tt--failed 0)
(defun tt-ok (label cond)
  (unless cond (setq tt--failed (1+ tt--failed)))
  (message "%s %s" (if cond "PASS" "FAIL") label))
(defun tt-wait (pred secs)
  (let ((t0 (float-time)))
    (while (and (not (funcall pred)) (< (- (float-time) t0) secs))
      (accept-process-output nil 0.1))
    (funcall pred)))
(defconst tt-pk "dc4cd086cd7ce5b1832adf4fdd1211289880d2c7e295bcb0e684c01acee77c06")
(defconst tt-index-buf "*tendrl: Index — example kasten*")

;;; 1. identifier parsing
(tt-ok "ident: hex" (eq (tendrl--ident-kind (make-string 64 ?a)) 'hex))
(tt-ok "ident: coord" (eq (tendrl--ident-kind (concat "30041:" tt-pk ":example-about")) 'coord))
(tt-ok "ident: naddr" (eq (tendrl--ident-kind "naddr1qqrk27") 'naddr))
(with-temp-buffer
  (insert "see nostr:naddr1qqrk27rpd4cxcegzyrwye5yxe47wtvvr9t05lhgjzy5f3qxjcl3ft09su6zvqxkwua7qvqcyqqq82kq0dcjku here")
  (goto-char 12)
  (tt-ok "ident at point strips nostr:" (string-prefix-p "naddr1qqrk27" (or (tendrl-ident-at-point) ""))))

;;; 2. local fetch by naddr → content buffer, raw JSON apart
(tendrl-local-fetch "naddr1qqrk27rpd4cxcegzyrwye5yxe47wtvvr9t05lhgjzy5f3qxjcl3ft09su6zvqxkwua7qvqcyqqq82kq0dcjku")
(tt-ok "local fetch by naddr opens the content buffer" (tt-wait (lambda () (get-buffer tt-index-buf)) 8))
(when-let* ((b (get-buffer tt-index-buf)))
  (with-current-buffer b
    (tt-ok "buffer is read-only" buffer-read-only)
    (tt-ok "event view mode on" tendrl-event-view-mode)
    (tt-ok "raw event kept apart as tendrl-event" (eql (alist-get 'kind tendrl-event) 30040))
    (tt-ok "content buffer holds no JSON" (not (string-match-p "\"sig\"" (buffer-string))))
    (tt-ok "header names the source" (string-match-p "· local " header-line-format))
    (tt-ok "j opens the JSON buffer" (buffer-live-p (tendrl-event-json)))
    (with-current-buffer (format "*tendrl json: %s*" (substring (alist-get 'id tendrl-event) 0 12))
      (tt-ok "JSON buffer is pretty-printed and read-only"
             (and buffer-read-only (string-match-p "\"kind\": 30040" (buffer-string))))
      (tt-ok "JSON buffer uses the JSON mode" (eq major-mode tendrl-json-mode)))))

;;; 3. local fetch by hex id and by coordinate → the section, rendered + nostrdown resolved
(let ((about "a2f54d63752bdc43acb0d2dfd7c0f4a426b561fdb0e8d8b4eb9fe3c5a34d820c"))
  (tendrl-local-fetch about)
  (tt-ok "local fetch by hex id" (tt-wait (lambda () (get-buffer "*tendrl: Index — example kasten — about*")) 8))
  (when-let* ((b (get-buffer "*tendrl: Index — example kasten — about*")))
    (with-current-buffer b
      (tt-ok "content rendered, not JSON" (string-match-p "{{ref:conventions|Conventions}}" (buffer-string)))
      (tt-ok "nostrdown-mode on in the event buffer" nostrdown-mode)
      (tt-ok "coordinate threaded for sibling resolution" (equal nostrdown-coord (concat "30041:" tt-pk ":example-about")))
      (tt-ok "refs resolve against the containing publication"
             (tt-wait (lambda () (and nostrdown--status (string-prefix-p "2/" nostrdown--status))) 8))
      (kill-buffer b)))
  (tendrl-local-fetch (concat "30041:" tt-pk ":example-about"))
  (tt-ok "local fetch by coordinate" (tt-wait (lambda () (get-buffer "*tendrl: Index — example kasten — about*")) 8)))

;;; 4. missing event reports, does not open
(let ((n (length (buffer-list))))
  (tendrl-local-fetch (make-string 64 ?f))
  (accept-process-output nil 1)
  (tt-ok "unknown id opens nothing" (= n (length (buffer-list)))))

;;; 5. relay fetch — engine path only in Auto mode; nak path always tried
(let (mode)
  (tendrl-request "GET" "/api/v1/network/status" nil
                  (lambda (_ data) (setq mode (or (alist-get 'mode data) "?"))) t)
  (message "network mode: %s" mode)
  (if (equal mode "auto")
      (progn
        (kill-buffer tt-index-buf)
        (tendrl-relay-fetch "naddr1qqrk27rpd4cxcegzyrwye5yxe47wtvvr9t05lhgjzy5f3qxjcl3ft09su6zvqxkwua7qvqcyqqq82kq0dcjku")
        (tt-ok "engine relay fetch returns" (tt-wait (lambda () (get-buffer tt-index-buf)) 30))
        (when-let* ((b (get-buffer tt-index-buf)))
          (with-current-buffer b (tt-ok "source says relays" (string-match-p "· relays" header-line-format)))))
    (message "SKIP engine relay fetch (network mode %s)" mode)))
(tendrl-relay-fetch "nevent1qqsd24n4r3vavztcwfc7p75je4z384lrntx3sr7chtdj0lqmy05vdasp0g9f8" t)
(let ((got (tt-wait (lambda () (get-buffer "*tendrl: Nostrdown*")) 30)))
  (message "nak relay fetch of the spec event: %s" (if got "arrived" "nothing (network)"))
  (when got (with-current-buffer "*tendrl: Nostrdown*"
              (tt-ok "nak source labelled" (string-match-p "· nak" header-line-format))
              (tt-ok "markdown event content rendered" (string-match-p "^# Nostrdown" (buffer-string))))))

(message "%s" (if (zerop tt--failed) "ALL PASS" (format "%d FAILED" tt--failed)))
(kill-emacs (if (zerop tt--failed) 0 1))
;;; tendrl-test.el ends here
