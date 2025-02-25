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
// }
// Environment
// {
//     "REDIS_URL": "redis://localhost:6379",
//     "TOKEN": "test",
//     "NUMBER": 3,
// }
{} == {}
[{}] == [{}]
[] == []
5 == 5
5 + 4 == 9
5 / 2.5 == 2
2 > 1
2.5 > 2
2.5 == 2.5
2 + 2 * 2 == 6
(2 + 2) * 2 == 8
(2+2)*2==8
true // false // check if comments work too
2 == /* Hey there */ 2
null == null
!!true == true
request.body.id == 1 
!true == false
"Hello"[0] == "H"
$NUMBER == 3
numbers[$NUMBER] == 4
request.headers["user-agent"] == "Tel"
request . headers [ "user-agent"]  == "Tel"
$REDIS_URL == "redis://localhost:6379"
$REDIS_URL + "/hello" == "redis://localhost:6379/hello"
numbers[0] == 1
numbers[1] == 2
[] + [] == []
{} + {} == {}
[1] + [2] == [1, 2]
[2] + [1] == [2, 1]
{ "a": 5 } == { a: 5 }
{ a: 5 } + { b: 4 } == { a: 5, b: 4 }
{ a: 5 } + { a: 4 } == { a: 4 }
{ a: 5 } + { a: 4, b: 3 } == { a: 4, b: 3 }
{ a: 5 } + { b: 4, a: 3 } == { b: 4, a: 3 }
{ a: 5 } + { b: 4, a: 3, c: 2 } == { b: 4, a: 3, c: 2 }
[1, 2, 3] + [4] == [1, 2, 3, 4]
"Hello" + "World" == "HelloWorld"
"Hello" + 5 == "Hello5"
"Hello" + 5.0 == "Hello5"
"Hello" + 5.1 == "Hello5.1"
["Hello"] + ["World"] == ["Hello", "World"]
"Hello" + null == "Hellonull"
!false == true
(true && true) == true
(true && false) == false
(false && true) == false
(false && false) == false
(true || true) == true
(true || false) == true
(false || true) == true
(false || false) == false
(5 + 5) * 2 == 20
5 + 5 * 2 == 15
5.0 + 5 == 10.0
5 + 5.0 == 10.0
5 + 5 == 10
request.body.name == "John Doe"
request.trololo.lololo == null
numbers[0] == 1
request["headers"]["user-agent"] == "Tel"
request.headers["user-agent"] == "Tel"
request["body"].id == 1
request["body"] == { id: 1, name: "John Doe" }
5.toString() == "5"
"hello".startsWith("h") == true
"hello".startsWith("nope") == false
$NUMBER + $NUMBER == 6
(true ? "halo" : true ? "halo" : "tres") == "halo"
(true ? 5 : 1) == 5
(true ? 5 : 1 == 5) == 5
(false ? 5 : 1) == 1
5 + "Hello".length == 10
{ a: $NUMBER == 3 ? 5 : 1 } == { a: 5 }
{ a: $NUMBER == 3 ? 5 : 1, b: 2 } == { a: 5, b: 2 }
"Hello".length + 5 == 10
[1, 2, 3].length + 0 == 3
[].length + 0 == 0
"Hello".startsWith("H") == true
"5.0".toNumber() == 5.0
"5.0".toNumber() == 5.0
"5".toNumber() == 5
5.4.round() == 5
5.4.round().round() == 5
5.6.floor() == 5
5.2.ceil() == 6
null.toString() == "null"
5.type() == "number"
"Hello".type() == "string"
[1, 2, 3].type() == "array"
{ a: 5 }.type() == "object"
true.type() == "boolean"
false.type() == "boolean"
5.0.type() == "number"
null.type() == "null"
(nothing || "hello") == "hello"
(request || "hello") != "hello"
5.4.isInteger == false
5.0.isInteger == true
5.isInteger == true
-9 * 2 == -18
"Hello".toUpperCase() == "HELLO"
"World".toLowerCase() == "world"
"".isEmpty() == true
" ".isEmpty() == false
" ".trim().isEmpty() == true
"Hello World".contains("Hello") == true
"Hello World".contains(" World") == true
[1, 2, 3].contains(1) == true
[1, 2, 3].contains(4) == false
[1, 2, 3][0].type() == "number"
numbers.contains(5) == true
$REDIS_URL + "/hello" == "redis://localhost:6379/hello"
numbers[0] + numbers[1] == 3
10 % 3 == 1
10 % 2 == 0
[1, 2, 3, 4].join("-") == "1-2-3-4"
[1, 2, 3, 4].join(", ") == "1, 2, 3, 4"
5.abs() == 5
-5.abs() == -5
(-5).abs() == 5
(nothing != null ? "OK" : "NOPE") == "NOPE"
(false ? "one" : true ? "two" : "three") == "two"
(true ? "one" : true ? "two" : "three") == "one"
2e-1 == 0.2
2e+1 == 20
"\u1114" == "ᄔ"
"hello".startsWith("he") == true
"hello".endsWith("ello") == true
3.14159.sin() == 0.00000265358979335273
3.14.isInteger() == false
4.0.sqrt() == 2.0
4.0.isOdd() == false
4.0.isEven()
(nothing || "Hello") == "Hello"
{} > [] == false
{} < [] == false 
"aaa" > "aa" == true
"a" < "b"
"hello_world".replace("world") == "hello_"
"hello_world".stripPrefix("hello") == "_world"
"hello_world".stripSuffix("_world") == "hello"
"hello_world".toUpperCase() == "HELLO_WORLD"
"Hello_World".toLowerCase() == "hello_world"
"image.jpg".endsWith(".jpg") == true
"image.jpg".endsWith(".png") == false
"hello world".replace("world", "there") == "hello there"
"image.png".stripFileExtension() == "image"
"image.jpg".stripFileExtension() == "image"
"image".stripFileExtension() == "image"
"abc".toCodePoints() == [97, 98, 99]
[97, 98, 99].fromCodePoints() == "abc"
"cześć".toCodePoints() == [99, 122, 101, 197, 155, 196, 135]
[99, 122, 101, 197, 155, 196, 135].fromCodePoints() == "cześć"
"FFAC".fromHexString() == [255, 172]
[250, 204, 21].toHexString() == "facc15"
"dHVyYm9mdXJv".fromBase64String().fromCodePoints() == "turbofuro"
"dHVyYm9mdXJv".fromBase64String() == [116, 117, 114, 98, 111, 102, 117, 114, 111]
"1,2,3".split(",") == ["1", "2", "3"]
["1", "2", "3"].join(", ") == "1, 2, 3"
{}.isEmpty()
[].isEmpty()
{ a: 5, } == { a: 5 }
[1, 2, 3,] == [1, 2, 3]
-5.sign() == -1
(-5).sign() == -1
(-5.7).truncate() == -5
-5.7.floor() == -5
(-5.7).floor() == -6
5.7.floor() == 5
// "false".toBoolean() == false
// "Hello".toBoolean() == true