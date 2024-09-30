// Storage
// {
//     "request": {
//         "headers": {
//             "access-control-allow-origin": "*",
//             "user-agent": "Tel",
//         },
//         "body": {
//             "id": 1,
//             "name": "John Doe"
//         }
//     },
//     "numbers": [1, 2, 3, 4, 5],
//     "anything": any
// }
// Environment
// {
//     "REDIS_URL": "redis://localhost:6379",
//     "TOKEN": "test",
//     "NUMBER": 3,
// }
// request["body"] CHECK { "id": 1, "name": "John Doe" }
{} CHECK {}
[] CHECK []
[a, "wow", 444] CHECK [null, "wow", 444]
[a, "wow", 444][1] CHECK "wow"
$NUMBER CHECK 3
[1] + [2] CHECK array
5 == 5 CHECK boolean
5 + 4 == 9 CHECK boolean
null == null CHECK boolean
"Hello"[0] CHECK "H"
numbers[$NUMBER] CHECK 4
request.headers["user-agent"] CHECK "Tel"
$REDIS_URL == "redis://localhost:6379" CHECK boolean
$REDIS_URL + "/hello" == "redis://localhost:6379/hello" CHECK boolean
numbers[0] == 1 CHECK boolean
numbers[1] == 2 CHECK boolean
[] + [] == [] CHECK boolean
{} + {} CHECK object
{ a: 5 } + { b: 4 } CHECK object
{ a: 5 } + { a: 4 } CHECK object
{ a: 5 } + { a: 4, b: 3 } CHECK object
{ a: 5 } + { b: 4, a: 3 } CHECK object
{ a: 5 } + { b: 4, a: 3, c: 2 } CHECK object
[1, 2, 3] + [4] CHECK array
(true ? "halo" : true ? "halo" : "tres") CHECK "halo"
5 / 2.5 CHECK number
"Hello" + "World" CHECK string
"Hello" + 5 CHECK string
"Hello" + 5.0 CHECK string
"Hello" + 5.1 CHECK string
["Hello"] + ["World"] CHECK array
"Hello" + null CHECK string
!true CHECK boolean
!false CHECK boolean
(true && true) CHECK boolean
(true && false) CHECK boolean
(false && true) CHECK boolean
(false && false) CHECK boolean
(true || true) CHECK boolean
(true || false) CHECK boolean
(false || true) CHECK boolean
(false || false) CHECK boolean
(5 + 5) * 2 CHECK number
5 + 5 * 2 CHECK number
5.0 + 5 CHECK number
5 + 5.0 CHECK number
5 + 5 CHECK number
request.body.id CHECK 1
request.body.name CHECK "John Doe"
request.trololo.lololo CHECK null
numbers[0] CHECK 1
request["headers"]["user-agent"] CHECK "Tel"
5.toString() CHECK string
"hello".startsWith("h") CHECK boolean
"hello".startsWith("nope") CHECK boolean
$NUMBER + $NUMBER CHECK number
(true ? 5 : 1) CHECK 5
(false ? 5 : 1) CHECK 1
5 + "Hello".length CHECK number
"Hello".length + 5 CHECK number
"Hello".startsWith("H") CHECK boolean
"5.0".toNumber() CHECK number
"5.0".toNumber() CHECK number
"5".toNumber() CHECK number
5.4.round() CHECK number
5.6.floor() CHECK number
5.2.ceil() CHECK number
null.toString() CHECK string
5.type() CHECK "number"
"Hello".type() CHECK "string"
[1, 2, 3].type() CHECK "array"
{ a: 5 }.type() CHECK "object"
true.type() CHECK "boolean"
false.type() CHECK "boolean"
5.0.type() CHECK "number"
null.type() CHECK "null"
5.4.isInteger CHECK false
5.0.isInteger CHECK true
5.isInteger CHECK true
-9 * 2 CHECK number
"Hello".toUpperCase() CHECK string
"World".toLowerCase() CHECK string
"".isEmpty() CHECK boolean
" ".isEmpty() CHECK boolean
" ".trim().isEmpty() CHECK boolean
"Hello World".contains("Hello") CHECK boolean
"Hello World".contains(" World") CHECK boolean
[1, 2, 3].contains(1) CHECK boolean
[1, 2, 3].contains(4) CHECK boolean
[1, 2, 3][0].type() CHECK "number"
numbers.contains(5) CHECK boolean
$REDIS_URL + "/hello" CHECK string
numbers[0] + numbers[1] CHECK number
10 % 3 CHECK error
10 % 2 CHECK error
[1, 2, 3, 4].join("-") CHECK string
[1, 2, 3, 4].join(", ") CHECK string
5.abs() CHECK number
-5.abs() CHECK number
(-5).abs() CHECK number
nothing != null ? "OK" : "NOPE" CHECK "OK" | "NOPE"
(nothing != null ? "OK" : "NOPE").type() CHECK "string"
nothing != null ? [] + "wtf" : 5 CHECK error | 5
anything + 5 CHECK number
anything + [] CHECK array
anything + {} CHECK object
null == anything CHECK boolean
1.0.sin() CHECK number
1.0.isInteger() CHECK boolean
4.0.sqrt() CHECK number
4.0.isOdd() CHECK boolean
"Hello World".stripPrefix("Hello ") CHECK string
"Hello World".stripSuffix(" World") CHECK string
"Hello World".stripPrefix("Hello ").stripSuffix(" World") CHECK string
"Hello".endsWith("ello") CHECK boolean
anything CHECK any
("/Users/r/Downloads/" + (anything || "")) CHECK string
(nothing || "Hello") CHECK string
{} > [] CHECK boolean
{} < [] CHECK boolean
"aaa" > "aa" CHECK boolean
(5 || "Hello").type() CHECK "number" | "string"