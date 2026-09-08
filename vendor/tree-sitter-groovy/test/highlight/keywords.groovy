class Foo { }
// <- keyword

trait T { }
// <- keyword

interface I { }
// <- keyword

enum E { A }
// <- keyword

record R(int x)
// <- keyword

package com.example
// <- keyword

import java.util.List
// <- keyword

def x = 5
// <- keyword

var y = 6
// <- keyword

sealed class S { }
// <- keyword

non-sealed class N permits A { }
// <- keyword

sealed class P permits B { }
//               ^ keyword

pipeline { }
// <- keyword

if (x) y
// <- keyword.control

while (x) y
// <- keyword.control

for (i in xs) y
// <- keyword.control

return x
// <- keyword.control

throw e
// <- keyword.control

try { x }
// <- keyword.control

switch (x) { case 1 -> y }
// <- keyword.control

yield 42
// <- keyword.control
