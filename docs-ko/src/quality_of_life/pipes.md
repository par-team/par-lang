# 파이프

파이프는 연쇄된 연산의 가독성을 유지해 주는 문법 설탕이다. Par에 새로운 기능을 추가하지는 않지만, 임시 변수를 만들지 않고도 '이것 다음에 저것' 형태의 코드를 왼쪽에서 오른쪽으로 작성할 수 있다.

## 파이프의 첫인상

다음 헬퍼 함수를 사용해 정수 연산을 수행한다고 해 보자.

```par
module Main

import @core/Nat

def Add    = [m: Nat, n: Nat] m + n
def Double = [n: Nat] n + n
def Square = [n: Nat] n * n
```

파이프가 없다면 함수 호출을 괄호 안에 중첩시켜서 작성해야 한다.

```par
let result = Square(Add(3, Double(4)))  // = 121
```

파이프를 사용하면 같은 연산을 자연스럽게 읽을 수 있는 순서로 작성할 수 있다.

```par
let result = 3
  -> Add(Double(4))
  -> Square
```

파이프 연산자는 좌변의 값을 우변에 오는 함수에 **첫 번째 인자**로 전달한다. 이외에는 아무런 차이도 없으며, 위의 두 코드는 정확히 같은 동작을 한다.

## 식 문법의 의미론

다음과 같은 코드를 보면...

```par
value -> Func(args)
```

다음과 같이 읽으면 된다.

```par
Func(value, args)
```

`(args)`나 `.case { ... }`처럼 `->Func`까지를 연산의 한 단위라고 생각하는 것이 도움이 된다. 위의 예제를 명확하게 구분하고 싶다면 `value ->Func (args)`와 같이 공백을 추가해도 된다.

```par
Account.Lookup(name)
  ->Auth.Check
  .case {
    .ok user   => user.balance
    .err _     => 0
  }
```

값을 첫 번째 인자가 아니라 다른 인자로 전달하고 싶다면, 함수 호출을 중괄호로 묶어서 묶인 식 전체가 한 단위의 연산이 되도록 하면 된다.

```par
value -> {Func(arg1)}(arg3)  // == Func(arg1, value, arg3)
```

`3`, `"text"`, `*(1, 2, 3)`, `<<1 2 3>>` 따위의 이름이나 값 뒤에는 파이프나 다른 후위 연산자를 직접 연쇄할 수 있는 한편, 복합 식이나 생성 식의 경우에는 중괄호를 사용해야 한다.

```par
{if { ready => value, else => fallback }}->Func
{.true!}.case { .true! => yes, .false! => no }
n->{box [x] x + n}
```

위의 중괄호는 후위 연산자를 적용하기 전에 중괄호 안의 식 전체를 하나의 값으로 묶는 역할을 한다. 중위 연산자를 사용한 식도 이 방법을 사용해 `{1 + 2}->Func`와 같이 작성해야 하며, 중괄호를 생략한 `1 + 2->Func`의 경우에는 `Func`가 `2`에만 적용된다.

## 명령 형태의 파이프

프로세스 문법에서는 모든 명령이 좌변의 변수를 '사용'한 뒤 명령의 결과를 같은 변수에 돌려놓는다. 파이프 역시 같은 규칙을 따르므로 임의의 함수를 내장 명령처럼 다루는 것이 가능하다.

```par
module Main

import {
  @core/List
  @core/Option
  @core/Nat
}

dec Push : [List<Nat>, Nat] List<Nat>
def Push = [stack, value] .item(value) stack

let stack = *(1, 2, 3)
stack->Push(4)
// let stack = Push(stack, 4)와 완전히 일치함
```

파이프를 사용하면 구조 분해 명령도 편리하게 작성할 수 있다. 리스트의 첫 원소와 나머지를 분리하는 `Pop` 함수를 사용해 보자.

```par
dec Pop : [List<Nat>] (Option<Nat>) List<Nat>
def Pop = [list] list.case {
  .end! => (.none!) .end!,
  .item(x) xs => (.some x) xs,
}

let numbers = *(10, 20, 30)
numbers->Pop[top]
// top     : Option<Nat>
// numbers : List<Nat>  -- *(20, 30)이 되었음
```

위의 명령 역시 `let (top) numbers = Pop(numbers)`를 보기 좋게 바꾼 것과 같다.

파이프 명령이 값을 **첫 번째** 인자로 전달하기 때문에 익숙한 `value.operation` 꼴의 명령의 나열을 확장하면서 언어 규칙에 예외가 생기지 않은 것이다.

## 정리

- 파이프를 사용해 Par의 평가 규칙을 유지하면서 연산을 왼쪽에서 오른쪽으로 표현할 수 있다. 파이프는 기존과 같은 중첩 호출로 변환된다.
- 파이프는 다른 문법과 잘 어울리기 때문에, `case`나 선택 등 다른 명령과 자연스럽게 섞어서 나열할 수 있다.
- 첫 인자를 제외한 다른 자리에 전달할 때는 중괄호로 원하는 위치를 지정할 수 있다.

`tmp` 같은 이름의 변수를 만들고 나서 바로 다음 함수로 전달하게 되는 일이 잦다면, 파이프 하나로 임시 변수 없이 가독성도 챙길 수 있다.
