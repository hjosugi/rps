# Int32 arithmetic that overflows is redone in Double, not wrapped.
2147483647 + 1
2147483647 * 2
-2147483647 - 2
(2147483647 + 1).GetType().FullName
(2147483646 + 1).GetType().FullName
2147483648
(2147483648).GetType().FullName
