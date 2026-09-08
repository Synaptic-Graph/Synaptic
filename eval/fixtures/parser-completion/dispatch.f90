module dispatch
  implicit none
  type base
  contains
    procedure :: run => base_run
  end type
  type, extends (base) :: child
  contains
    procedure :: run => child_run
  end type
  interface pick
    module procedure single_value, double_value, vector_value
  end interface
contains
  integer function base_run(self)
    class(base) :: self
    base_run = 1
  end function
  integer function child_run(self)
    class(child) :: self
    child_run = 2
  end function
  integer function single_value(x)
    real(kind=4) :: x
    single_value = 4
  end function
  integer function double_value(x)
    real(kind=8) :: x
    double_value = 8
  end function
  integer function vector_value(x)
    real(kind=8), dimension(:) :: x
    vector_value = size(x)
  end function
  subroutine polymorphic(item)
    class(base) :: item
    if (item%run() /= 2) stop 1
  end subroutine
end module

program check_dispatch
  use dispatch
  implicit none
  type(child) :: item
  real(kind=8), dimension(3) :: values
  if (item%run() /= 2) stop 2
  if (pick(1.0_8) /= 8) stop 3
  if (pick(values) /= 3) stop 4
  call polymorphic(item)
end program
